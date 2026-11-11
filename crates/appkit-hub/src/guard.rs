//! The guard: every request passes these eight steps, in this order, before any route runs.

use crate::*;

// ── the guard ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// The static page of the app, no credential.
    Shell,
    /// `hello`: version and abilities, no credential and no data.
    Hello,
    /// Pairing code for a session token.
    Exchange,
    /// Mints a pairing code, for a local program with the control secret.
    ControlPair,
    /// Everything else under `/api/`: a session is needed.
    Api,
    /// Not ours.
    Other,
}

/// What an app tells the guard about its routes: paths and nothing else. The checks, their order
/// and the classes are the guard's own, so an app cannot loosen one.
#[derive(Debug, Clone)]
pub struct GuardRules {
    /// The path of `hello`: version and abilities, no credential and no data.
    hello_path: String,
    /// POST paths that are counted by the coarse limit of `ops` (a read that is a POST, or a write
    /// that only its body tells from a dry run) and not by the strict one of the other mutations.
    coarse_post_paths: Vec<String>,
    /// The one address besides this origin that the page may fetch from (`https://api.github.com`); none:
    /// this origin only.
    connect_src: Option<String>,
    /// POST paths that take a file as it is: any size, any type, and as long as it takes. A session and the origin are
    /// asked as for any other route; the route itself has to stream the body and give up on one that stalls.
    upload_paths: Vec<String>,
    /// A second page of the app that can be installed (a PWA for a phone), below its own path prefix.
    installable: Option<Installable>,
    /// Where the hub keeps its paired devices (`devices.json` in the settings); none: no device keys, only codes.
    devices: Option<std::path::PathBuf>,
    /// The name of this machine in the tailnet (`with_tailnet_name`); none: loopback names only.
    tailnet: Option<String>,
    /// The routes a paired device may reach (`with_device_route`), each with the level it needs; every other route
    /// below `/api/` refuses a device.
    device_routes: Vec<(String, Level)>,
    /// The level of a newly paired device (`with_device_default_level`).
    device_default: Level,
    /// Web Push to devices (`with_push`).
    push: Option<std::sync::Arc<crate::push::Push>>,
}

/// The routes of device keys (`with_devices`): pair with a code and a public key, ask a challenge, open a session.
pub const DEVICE_PAIR_PATH: &str = "/auth/device/pair";
pub const DEVICE_CHALLENGE_PATH: &str = "/auth/device/challenge";
pub const DEVICE_SESSION_PATH: &str = "/auth/device/session";
/// The routes of the PC for its devices, with the control secret: the list, and revoking one.
pub const CONTROL_DEVICES_PATH: &str = "/api/control/devices";
pub const CONTROL_REVOKE_PATH: &str = "/api/control/devices/revoke";
/// The route of the PC that sets the level of a device, with the control secret.
pub const CONTROL_LEVEL_PATH: &str = "/api/control/devices/level";
/// The header of a trigger: `<challenge>.<signature>` of the device's confirmation key over `confirm_message`.
pub const CONFIRM_HEADER: &str = "x-device-confirm";

/// A page below `prefix` (`/m`) that a browser can install: it is a shell like `/`, and its policy also lets the
/// page name its manifest and its service worker, and make one more Trusted Types policy (`worker_policy`), the one
/// the page turns the address of its worker into a script URL with. Everything else of the policy is the shell's.
#[derive(Debug, Clone)]
struct Installable {
    prefix: String,
    worker_policy: String,
    /// The page may record from the microphone (`with_installable_microphone`).
    microphone: bool,
}

/// The first parts of a path that are the core's: an installable page cannot live there.
const RESERVED_PREFIXES: [&str; 4] = ["/api", "/assets", "/auth", "/index.html"];

/// Is `prefix` one plain path part after a slash: lower-case letters, digits and `-`, at most 32 of them?
fn plain_prefix(prefix: &str) -> bool {
    prefix.strip_prefix('/').is_some_and(|part| {
        (1..=32).contains(&part.len())
            && !part.starts_with('-')
            && part
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    })
}

/// Is `name` a name for a Trusted Types policy that cannot widen the policy list: lower-case letters, digits and `-`,
/// starting with a letter, not a keyword of the directive and not the shell's own policy?
fn plain_policy_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && name != "svelte-trusted-html"
        && name != "default"
        && name != "none"
}

/// The routes of the core itself: an app cannot name one of them as its own.
const CORE_PATHS: [&str; 13] = [
    crate::push::PUSH_KEY_PATH,
    crate::push::PUSH_SUBSCRIBE_PATH,
    crate::push::PUSH_UNSUBSCRIBE_PATH,
    "/api/control/pair",
    CONTROL_DEVICES_PATH,
    CONTROL_REVOKE_PATH,
    CONTROL_LEVEL_PATH,
    "/api/vaults",
    "/api/vaults/add",
    "/api/vaults/remove",
    "/api/vaults/select",
    "/api/vaults/update",
    "/auth/exchange",
];

/// Is `path` a plain route below `/api/`: one that the guard will ask a session for?
fn plain_api_path(path: &str) -> bool {
    path.len() > "/api/".len()
        && path.starts_with("/api/")
        && !path.ends_with('/')
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'_' | b'-' | b'.'))
        && path
            .split('/')
            .skip(1)
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

/// Is `origin` exactly `https://` + a host name (labels of lower-case letters, digits and `-`, at least
/// two of them, none empty or starting or ending with `-`, the last one not all digits) + an optional port?
fn plain_https_origin(origin: &str) -> bool {
    let Some(rest) = origin.strip_prefix("https://") else {
        return false;
    };
    let (host, port) = match rest.split_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (rest, None),
    };
    let port_ok = port.is_none_or(|p| {
        (1..=5).contains(&p.len())
            && p.bytes().all(|b| b.is_ascii_digit())
            && p.parse::<u16>().is_ok_and(|n| n > 0)
    });
    let labels: Vec<&str> = host.split('.').collect();
    port_ok
        && labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty()
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
        // A host of digits only is an address, not a name.
        && labels
            .last()
            .is_some_and(|tld| tld.bytes().any(|b| b.is_ascii_lowercase()))
}

impl GuardRules {
    /// The rules of an app.
    ///
    /// The paths are the app's own routes and are checked here, once, when the hub is made: a mistake in
    /// them would open a route to anyone (`hello` needs no session) or hide the limit of another, so it
    /// is a fault of the program that makes the hub and ends it at once, not a thing to be found at run
    /// time.
    ///
    /// # Panics
    ///
    /// When `hello_path` or one of the `coarse_post_paths` is not a plain route below `/api/` (letters,
    /// digits, `/ _ - .`, no empty or dot part, no trailing slash), or when `hello_path` is one of the
    /// routes of the core (`/api/control/pair`, `/api/vaults*`).
    #[must_use]
    pub fn new(
        hello_path: impl Into<String>,
        coarse_post_paths: impl IntoIterator<Item = String>,
    ) -> Self {
        let hello_path = hello_path.into();
        assert!(
            plain_api_path(&hello_path),
            "the path of hello must be a plain route below /api/, not {hello_path:?}: hello needs no session"
        );
        assert!(
            !CORE_PATHS.contains(&hello_path.as_str()),
            "the path of hello is a route of the core: {hello_path:?}"
        );
        let coarse_post_paths: Vec<String> = coarse_post_paths.into_iter().collect();
        for path in &coarse_post_paths {
            assert!(
                plain_api_path(path),
                "a coarse POST path must be a plain route below /api/, not {path:?}"
            );
        }
        Self {
            hello_path,
            coarse_post_paths,
            upload_paths: Vec::new(),
            connect_src: None,
            installable: None,
            devices: None,
            tailnet: None,
            device_routes: Vec::new(),
            device_default: Level::Create,
            push: None,
        }
    }

    /// Let paired devices subscribe to Web Push of this hub (`src/push.rs`): adds `/api/push/key`,
    /// `/api/push/subscribe` and `/api/push/unsubscribe`, released for devices at the level `read` and for nobody
    /// else (a session of the PC gets `403 device_required` there). The app keeps a clone of `push` to tell devices
    /// what happened (`Push::notify`). A revoked device's subscription goes with it.
    ///
    /// # Panics
    ///
    /// When the app keeps no devices (`with_devices` first), or when it is called twice.
    #[must_use]
    pub fn with_push(mut self, push: std::sync::Arc<crate::push::Push>) -> Self {
        assert!(self.devices.is_some(), "with_push needs with_devices first");
        assert!(self.push.is_none(), "with_push is called once");
        self.push = Some(push);
        for path in [
            crate::push::PUSH_KEY_PATH,
            crate::push::PUSH_SUBSCRIBE_PATH,
            crate::push::PUSH_UNSUBSCRIBE_PATH,
        ] {
            self.device_routes.push((path.to_owned(), Level::Read));
        }
        self
    }

    /// The push of the hub, if the app has one.
    #[must_use]
    pub fn push(&self) -> Option<&std::sync::Arc<crate::push::Push>> {
        self.push.as_ref()
    }

    /// Release one route of the app for paired devices with the level it needs (`MOB-02`, `MOB-03`). A session of a
    /// device reaches only the routes released this way; every other route below `/api/` (the core's `/api/vaults*`
    /// among them) answers it `403 not_for_devices`, whatever the method, before the route runs. Without a call, a
    /// device reaches nothing: fail closed. A device below the level gets `403 level_too_low`. A route of
    /// [`Level::Trigger`] also needs, for each request, the confirmation of the device (`x-device-confirm`, see
    /// `devices::confirm_message`); without it, or with a wrong or spent one, `403 confirm_required`. The route still
    /// cuts its answer by class for a device (`Caller`).
    ///
    /// # Panics
    ///
    /// When `path` is not a plain route below `/api/`, or is `hello` or a route of the core, or is released twice.
    #[must_use]
    pub fn with_device_route(mut self, path: impl Into<String>, level: Level) -> Self {
        let path = path.into();
        assert!(
            plain_api_path(&path),
            "a device route must be a plain route below /api/, not {path:?}"
        );
        assert!(
            path != self.hello_path && !CORE_PATHS.contains(&path.as_str()),
            "a device route must be a route of the app, not {path:?}"
        );
        assert!(
            self.device_level(&path).is_none(),
            "a device route is released once: {path:?}"
        );
        self.device_routes.push((path, level));
        self
    }

    /// The level a device needs for `path`; `None`: the route is not released for devices.
    #[must_use]
    pub fn device_level(&self, path: &str) -> Option<Level> {
        self.device_routes
            .iter()
            .find(|(p, _)| p == path)
            .map(|(_, l)| *l)
    }

    /// The level a newly paired device gets (the kit's default is [`Level::Create`]).
    ///
    /// # Panics
    ///
    /// For [`Level::Trigger`]: triggering is granted on the PC, one device at a time, never by default.
    #[must_use]
    pub fn with_device_default_level(mut self, level: Level) -> Self {
        assert!(
            level < Level::Trigger,
            "trigger is never the default level of a device"
        );
        self.device_default = level;
        self
    }

    /// The level a newly paired device gets.
    #[must_use]
    pub fn device_default_level(&self) -> Level {
        self.device_default
    }

    /// Let a phone reach the hub through Tailscale Serve (`tailscale serve` forwards
    /// `https://<name>` to the loopback port of the hub; the hub still binds loopback only, `SRV-01`).
    /// The guard then also takes the `Host` `name` and the `Origin` `https://<name>`, exactly, and holds a
    /// request with that host to what a device may: no session of a pairing code (`/auth/exchange` is refused),
    /// no route of the control secret, and every route below `/api/` only with a session of a paired device.
    /// Pairing a device (`/auth/device/*`) is what this host is for. Needs `with_devices`.
    ///
    /// # Panics
    ///
    /// When `name` is not a plain host name (lower-case labels of letters, digits and `-`, at least two, the last
    /// not all digits; no port, no scheme), or when the app keeps no devices.
    #[must_use]
    pub fn with_tailnet_name(mut self, name: impl Into<String>) -> Self {
        let name = name.into();
        assert!(
            !name.contains(':') && plain_https_origin(&format!("https://{name}")),
            "the tailnet name must be a plain host name like pc.example.ts.net, not {name:?}"
        );
        assert!(
            self.devices.is_some(),
            "with_tailnet_name needs with_devices first: only devices come through the tailnet"
        );
        self.tailnet = Some(name);
        self
    }

    /// The name of this machine in the tailnet, if the app named one.
    #[must_use]
    pub fn tailnet_name(&self) -> Option<&str> {
        self.tailnet.as_deref()
    }

    /// Let devices pair once with a key of their own (`src/devices.rs`) and keep them in `path` (a file in the
    /// settings of the app, `devices.json`). Adds the routes `/auth/device/pair`, `/auth/device/challenge` and
    /// `/auth/device/session` (no session needed, like `/auth/exchange`, and counted by its limit) and, for the PC with
    /// the control secret, `/api/control/devices` and `/api/control/devices/revoke`.
    #[must_use]
    pub fn with_devices(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.devices = Some(path.into());
        self
    }

    /// The file of the devices, if the app keeps them.
    #[must_use]
    pub fn devices_file(&self) -> Option<&std::path::Path> {
        self.devices.as_deref()
    }

    /// Serve a second page that can be installed (a PWA) below `prefix` (`/m`): `/m/` and every file below it are
    /// asked for by the app's `Pages` as `/m/...`, as shells (no credential, `GET` only), with the shell's headers.
    /// Their policy also says `manifest-src 'self'; worker-src 'self'` and allows the Trusted Types policy
    /// `worker_policy` next to Svelte's: the page needs it to register its service worker under
    /// `require-trusted-types-for 'script'`, and should let it turn exactly one string, the worker's address, into a
    /// script URL. The policy of every other path stays as it is.
    ///
    /// # Panics
    ///
    /// When `prefix` is not one plain part (`/` + lower-case letters, digits, `-`), or is a part of the core
    /// (`/api`, `/assets`, `/auth`), or when `worker_policy` is not a plain name (lower-case letters, digits, `-`,
    /// starting with a letter) or is Svelte's own, `default` or `none`; or when it is called twice.
    #[must_use]
    pub fn with_installable_page(
        mut self,
        prefix: impl Into<String>,
        worker_policy: impl Into<String>,
    ) -> Self {
        let prefix = prefix.into();
        let worker_policy = worker_policy.into();
        assert!(
            plain_prefix(&prefix) && !RESERVED_PREFIXES.contains(&prefix.as_str()),
            "the prefix of an installable page must be one plain path part like /m, not {prefix:?}"
        );
        assert!(
            plain_policy_name(&worker_policy),
            "the worker policy must be a plain name of its own, not {worker_policy:?}"
        );
        assert!(
            self.installable.is_none(),
            "an app has at most one installable page"
        );
        self.installable = Some(Installable {
            prefix,
            worker_policy,
            microphone: false,
        });
        self
    }

    /// Let the installable page record from the microphone (a voice note): on its paths only, the permissions say
    /// `microphone=(self)` instead of `microphone=()`, and the policy adds `media-src 'self' blob:`, so the page can
    /// play back a recording it made itself before it sends it. The browser still asks the person; every other path
    /// keeps the microphone off.
    ///
    /// # Panics
    ///
    /// When the app has no installable page (`with_installable_page` first).
    #[must_use]
    pub fn with_installable_microphone(mut self) -> Self {
        let page = self
            .installable
            .as_mut()
            .expect("with_installable_microphone needs with_installable_page first");
        page.microphone = true;
        self
    }

    /// The prefix of the installable page, if the app has one (`/m`).
    #[must_use]
    pub fn installable_prefix(&self) -> Option<&str> {
        self.installable.as_ref().map(|i| i.prefix.as_str())
    }

    /// Is `path` the installable page or a file below it (`/m/`, `/m/sw.js`)? `/m` itself is not: relative
    /// addresses of the page would resolve against `/`.
    #[must_use]
    pub fn is_installable(&self, path: &str) -> bool {
        self.installable.as_ref().is_some_and(|i| {
            path.strip_prefix(i.prefix.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
        })
    }

    /// The installable page, when `path` is on it.
    fn installable_for(&self, path: &str) -> Option<&Installable> {
        if self.is_installable(path) {
            self.installable.as_ref()
        } else {
            None
        }
    }

    /// Let the page fetch from one more address, besides this origin (`connect-src 'self' <origin>`).
    ///
    /// This widens what the page can reach, so it is narrow on purpose: one origin, `https` only, a host
    /// name with a dot (no wildcard, no address, no path, no query, no scheme other than `https`), checked
    /// here when the hub is made. An app that does not call this keeps `connect-src 'self'`.
    ///
    /// # Panics
    ///
    /// When `origin` is not of the form `https://host[:port]` as above.
    #[must_use]
    pub fn with_connect_src(mut self, origin: impl Into<String>) -> Self {
        let origin = origin.into();
        assert!(
            plain_https_origin(&origin),
            "the extra connect-src must be https://host[:port] with a host name, not {origin:?}"
        );
        self.connect_src = Some(origin);
        self
    }

    /// A route that takes a file as it is (see `upload_paths`). It has to read the body as a stream, with
    /// `DefaultBodyLimit::disable()`, and end a transfer that stalls: the guard no longer does.
    ///
    /// # Panics
    ///
    /// When `path` is not a plain route below `/api/`, or is `hello` or a route of the core.
    #[must_use]
    pub fn with_upload_path(mut self, path: impl Into<String>) -> Self {
        let path = path.into();
        assert!(
            plain_api_path(&path),
            "an upload path must be a plain route below /api/, not {path:?}"
        );
        assert!(
            path != self.hello_path && !CORE_PATHS.contains(&path.as_str()),
            "an upload path must be a route of the app, not {path:?}"
        );
        self.upload_paths.push(path);
        self
    }

    /// Is `path` a route that takes a file as it is?
    #[must_use]
    pub fn is_upload(&self, path: &str) -> bool {
        self.upload_paths.iter().any(|p| p == path)
    }

    /// The extra address of `connect-src`, if the app named one.
    #[must_use]
    pub fn connect_src(&self) -> Option<&str> {
        self.connect_src.as_deref()
    }

    /// The path of `hello`.
    #[must_use]
    pub fn hello_path(&self) -> &str {
        &self.hello_path
    }

    fn is_coarse(&self, path: &str) -> bool {
        self.coarse_post_paths.iter().any(|p| p == path)
    }
}

pub fn classify(path: &str, rules: &GuardRules) -> Class {
    match path {
        "/" | "/index.html" => Class::Shell,
        p if p.starts_with("/assets/") => Class::Shell,
        p if rules.is_installable(p) => Class::Shell,
        p if p == rules.hello_path => Class::Hello,
        "/auth/exchange" => Class::Exchange,
        DEVICE_PAIR_PATH | DEVICE_CHALLENGE_PATH | DEVICE_SESSION_PATH
            if rules.devices.is_some() =>
        {
            Class::Exchange
        }
        "/api/control/pair" | CONTROL_DEVICES_PATH | CONTROL_REVOKE_PATH | CONTROL_LEVEL_PATH => {
            Class::ControlPair
        }
        p if p.starts_with("/api/") => Class::Api,
        _ => Class::Other,
    }
}

/// Another app opens the installable page: a share target, a link in a mail, a launcher. The browser says
/// `Sec-Fetch-Site: cross-site` then. Only that, only as the page itself (`GET`, a navigation to a document) and only
/// below the installable prefix, gets past the rule of `Sec-Fetch-Site`: the page is static and needs no credential,
/// what it loads afterwards comes from this origin, and every route of data keeps the rule. The desktop page is not
/// opened this way.
fn navigation_into_installable_page(
    rules: &GuardRules,
    parts: &axum::http::request::Parts,
    fetch_site: Option<&str>,
) -> bool {
    let header = |name: &str| parts.headers.get(name).and_then(|v| v.to_str().ok());
    matches!(fetch_site, Some("cross-site" | "same-site"))
        && parts.method == Method::GET
        && rules.is_installable(parts.uri.path())
        && header("sec-fetch-mode") == Some("navigate")
        && header("sec-fetch-dest") == Some("document")
}

pub fn method_allowed(class: Class, method: &Method) -> bool {
    match class {
        Class::Shell | Class::Hello => method == Method::GET,
        Class::Exchange | Class::ControlPair => method == Method::POST,
        Class::Api => method == Method::GET || method == Method::POST,
        // The fallback answers 404 to anything.
        Class::Other => true,
    }
}

pub fn error_body(status: StatusCode, code: &str, message: &str) -> Response {
    let body = json!({ "error": { "code": code, "message": message } });
    (status, axum::Json(body)).into_response()
}

/// A header that must be there at most once and be plain ASCII.
// The one answer to a header that is not is a refusal; there is nothing more to say about it.
#[allow(clippy::result_unit_err)]
pub fn single(headers: &HeaderMap, name: &HeaderName) -> Result<Option<String>, ()> {
    let mut values = headers.get_all(name).iter();
    let Some(first) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(());
    }
    first.to_str().map(|s| Some(s.to_owned())).map_err(|_| ())
}

pub fn bearer(headers: &HeaderMap) -> Option<String> {
    let value = single(headers, &header::AUTHORIZATION).ok()??;
    value.strip_prefix("Bearer ").map(str::to_owned)
}

pub fn is_json_content_type(headers: &HeaderMap) -> bool {
    let Ok(Some(value)) = single(headers, &header::CONTENT_TYPE) else {
        return false;
    };
    let value = value.to_ascii_lowercase();
    let mut parts = value.split(';').map(str::trim);
    if parts.next() != Some("application/json") {
        return false;
    }
    parts.all(|p| p == "charset=utf-8")
}

pub static SEC_FETCH_SITE: HeaderName = HeaderName::from_static("sec-fetch-site");

pub async fn guard<R: Runtime>(
    State(st): State<Arc<Core<R>>>,
    req: Request,
    next: Next,
) -> Response {
    let class = classify(req.uri().path(), &st.guard_rules);
    let page = st.guard_rules.installable_for(req.uri().path()).cloned();
    let mut response = judge(&st, class, req, next).await;
    set_page_headers(
        &mut response,
        class,
        st.guard_rules.connect_src(),
        page.as_ref().map(|p| p.worker_policy.as_str()),
        page.as_ref().is_some_and(|p| p.microphone),
    );
    response
}

pub async fn judge<R: Runtime>(st: &Core<R>, class: Class, req: Request, next: Next) -> Response {
    let (parts, body) = req.into_parts();
    let headers = &parts.headers;

    // 1. Host
    let remote = match single(headers, &header::HOST) {
        Ok(Some(host)) if st.policy.host_ok(&host) => st.policy.is_remote(&host),
        Ok(_) => return error_body(StatusCode::FORBIDDEN, "bad_host", "unknown host"),
        Err(()) => return error_body(StatusCode::BAD_REQUEST, "bad_header", "invalid Host header"),
    };

    let mutation = parts.method != Method::GET;

    // 2. Declared body size
    // (No header at all is fine: a GET, or a chunked body that the body
    // limit of the extractor bounds while it is read.)
    let upload = class == Class::Api && st.guard_rules.is_upload(parts.uri.path());
    let body_limit = match class {
        Class::Exchange | Class::ControlPair => st.limits.small_body_bytes,
        _ if upload => usize::MAX,
        _ => st.limits.max_body_bytes,
    };
    match single(headers, &header::CONTENT_LENGTH) {
        Ok(None) => {}
        Ok(Some(len)) => match len.parse::<usize>() {
            Ok(n) if n <= body_limit => {}
            Ok(_) => {
                let mut r =
                    error_body(StatusCode::PAYLOAD_TOO_LARGE, "too_large", "body too large");
                r.headers_mut()
                    .insert(header::CONNECTION, HeaderValue::from_static("close"));
                return r;
            }
            Err(_) => {
                return error_body(
                    StatusCode::BAD_REQUEST,
                    "bad_header",
                    "invalid Content-Length",
                );
            }
        },
        Err(()) => {
            return error_body(
                StatusCode::BAD_REQUEST,
                "bad_header",
                "invalid Content-Length",
            );
        }
    }

    // 3. Origin and Fetch Metadata
    let origin = match single(headers, &header::ORIGIN) {
        Ok(o) => o,
        Err(()) => {
            return error_body(
                StatusCode::BAD_REQUEST,
                "bad_header",
                "invalid Origin header",
            )
        }
    };
    if origin.as_deref().is_some_and(|o| !st.policy.origin_ok(o)) {
        return error_body(StatusCode::FORBIDDEN, "bad_origin", "unknown origin");
    }
    let fetch_site = match single(headers, &SEC_FETCH_SITE) {
        Ok(v) => v,
        Err(()) => {
            return error_body(
                StatusCode::BAD_REQUEST,
                "bad_header",
                "invalid Sec-Fetch-Site header",
            );
        }
    };
    if !fetch_site_ok(fetch_site.as_deref())
        && !navigation_into_installable_page(&st.guard_rules, &parts, fetch_site.as_deref())
    {
        return error_body(
            StatusCode::FORBIDDEN,
            "bad_fetch_site",
            "cross-site request",
        );
    }

    // 4. Method (after Origin: a foreign origin is told so whatever it tried)
    if !method_allowed(class, &parts.method) {
        return error_body(
            StatusCode::METHOD_NOT_ALLOWED,
            "bad_method",
            "method not allowed",
        );
    }

    // What is not one of our routes ends here, so that a route which is
    // later added to the router outside `/api/` cannot be open by accident.
    if class == Class::Other {
        return error_body(StatusCode::NOT_FOUND, "not_found", "no such route");
    }

    // Through the tailnet only what a device does: pair, prove itself, use its session. The control secret and a
    // session of a pairing code belong to this machine.
    if remote
        && (class == Class::ControlPair
            || (class == Class::Exchange && parts.uri.path() == "/auth/exchange"))
    {
        return error_body(
            StatusCode::FORBIDDEN,
            "not_on_tailnet",
            "this route is for the PC only",
        );
    }

    // 5. A mutation without an Origin is a program client, never a browser.
    if mutation && origin.is_none() && (fetch_site.is_some() || class == Class::Exchange) {
        return error_body(
            StatusCode::FORBIDDEN,
            "origin_required",
            "an Origin is required",
        );
    }

    // 6. Rate limit for mutations, before any credential. Only the body of `ops` tells a write
    // from a dry run, so the route has a coarse limit here for everything sent to it, and its
    // writes are counted again, strictly, once the handler has read the body.
    if mutation {
        let limit = if st.guard_rules.is_coarse(parts.uri.path()) {
            &st.ops_limit
        } else {
            &st.post_limit
        };
        if !limit.allow() {
            return too_many();
        }
    }

    // 7. The credential the route needs, and who it is (a route below `/api/` reads the `Caller`).
    let mut caller = None;
    let mut confirm_for = None;
    match class {
        Class::Shell | Class::Hello | Class::Exchange | Class::Other => {}
        Class::ControlPair => {
            let ok = bearer(headers)
                .and_then(|t| Secret::from_hex(&t, TOKEN_BYTES))
                .is_some_and(|t| t.matches(&st.control));
            if !ok {
                return error_body(
                    StatusCode::UNAUTHORIZED,
                    "unauthorized",
                    "credential required",
                );
            }
        }
        Class::Api => {
            let token = bearer(headers);
            let ok = token.as_deref().is_some_and(|t| st.sessions.check(t));
            if !ok {
                return error_body(StatusCode::UNAUTHORIZED, "unauthorized", "session required");
            }
            let device = token.as_deref().and_then(|t| st.sessions.device_of(t));
            if remote && device.is_none() {
                return error_body(
                    StatusCode::FORBIDDEN,
                    "device_required",
                    "through the tailnet only a paired device has a session",
                );
            }
            if let Some(id) = device.as_deref() {
                // A device reaches only what the app released for devices: fail closed (`MOB-02`).
                let Some(needed) = st.guard_rules.device_level(parts.uri.path()) else {
                    return error_body(
                        StatusCode::FORBIDDEN,
                        "not_for_devices",
                        "this route is not released for devices",
                    );
                };
                // At the level the device has now (the PC may have lowered it since the session began) (`MOB-03`).
                let has = st.devices.as_ref().and_then(|d| d.level_of(id));
                let Some(has) = has else {
                    return error_body(
                        StatusCode::UNAUTHORIZED,
                        "unauthorized",
                        "session required",
                    );
                };
                if has < needed {
                    return error_body(
                        StatusCode::FORBIDDEN,
                        "level_too_low",
                        "this device may not do this; the PC can raise its level",
                    );
                }
                if needed == Level::Trigger {
                    confirm_for = Some(id.to_owned());
                }
            }
            caller = Some(match device {
                Some(id) => Caller::Device(id),
                None => Caller::Pc,
            });
        }
    }

    // 8. A mutation carries JSON
    // (A file is what it is. A form cannot post here all the same: the route needs the bearer token.)
    if mutation && !upload && !is_json_content_type(headers) {
        return error_body(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "bad_content_type",
            "application/json required",
        );
    }

    // 9. A trigger of a device carries the confirmation of this very request, made on the device (`MOB-03`).
    let body = match confirm_for {
        Some(device) => match confirmed(st, &parts, &device, body).await {
            Ok(body) => body,
            Err(Unconfirmed::TooLarge) => {
                return error_body(StatusCode::PAYLOAD_TOO_LARGE, "too_large", "body too large");
            }
            Err(Unconfirmed::No) => {
                return error_body(
                    StatusCode::FORBIDDEN,
                    "confirm_required",
                    "confirm this on the device",
                );
            }
        },
        None => body,
    };

    let mut parts = parts;
    if let Some(caller) = caller {
        parts.extensions.insert(caller);
    }
    let req = Request::from_parts(parts, body);
    if upload {
        return next.run(req).await;
    }
    match tokio::time::timeout(st.limits.request_timeout, next.run(req)).await {
        Ok(response) => response,
        Err(_) => error_body(
            StatusCode::REQUEST_TIMEOUT,
            "timeout",
            "the request took too long",
        ),
    }
}

/// Why a trigger is not let through.
enum Unconfirmed {
    /// No confirmation, or one that does not hold for this request.
    No,
    /// The body is over the limit.
    TooLarge,
}

/// Read the body of a trigger (bounded like any body) and check the confirmation header against it; the body is given
/// back for the route.
async fn confirmed<R: Runtime>(
    st: &Core<R>,
    parts: &axum::http::request::Parts,
    device: &str,
    body: Body,
) -> Result<Body, Unconfirmed> {
    use sha2::Digest;
    let refuse = || Unconfirmed::No;
    let header = HeaderName::from_static(CONFIRM_HEADER);
    let Ok(Some(value)) = single(&parts.headers, &header) else {
        return Err(refuse());
    };
    let Some((challenge, signature)) = value.split_once('.') else {
        return Err(refuse());
    };
    let Ok(bytes) = axum::body::to_bytes(body, st.limits.max_body_bytes).await else {
        return Err(Unconfirmed::TooLarge);
    };
    let origin = parts
        .headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let path = parts
        .uri
        .path_and_query()
        .map_or_else(|| parts.uri.path().to_owned(), |p| p.as_str().to_owned());
    let digest: String = sha2::Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let ok = st.devices.as_ref().is_some_and(|d| {
        d.confirm(device, challenge, signature, |c| {
            crate::devices::confirm_message(
                origin,
                device,
                c,
                parts.method.as_str(),
                &path,
                &digest,
            )
        })
    });
    if ok {
        Ok(Body::from(bytes))
    } else {
        Err(refuse())
    }
}

pub fn too_many() -> Response {
    error_body(
        StatusCode::TOO_MANY_REQUESTS,
        "rate_limited",
        "too many requests",
    )
}

/// The policy of the page: this origin only, plus the one address an app named for `connect-src`.
#[cfg(test)]
fn shell_policy(connect_src: Option<&str>) -> HeaderValue {
    page_policy(connect_src, None, false)
}

/// The policy of a page; with `worker_policy` the one of the installable page (`with_installable_page`): the same,
/// plus its manifest, its service worker from this origin and the one more Trusted Types policy it names; with
/// `media` also `media-src 'self' blob:` (a page that records plays back what it recorded).
fn page_policy(connect_src: Option<&str>, worker_policy: Option<&str>, media: bool) -> HeaderValue {
    let connect = match connect_src {
        Some(origin) => format!("'self' {origin}"),
        None => "'self'".to_owned(),
    };
    let (install, trusted) = match worker_policy {
        Some(name) => (
            " manifest-src 'self'; worker-src 'self';",
            format!("svelte-trusted-html {name}"),
        ),
        None => ("", "svelte-trusted-html".to_owned()),
    };
    let media = if media && worker_policy.is_some() {
        " media-src 'self' blob:;"
    } else {
        ""
    };
    // Nothing is allowed that the page does not name. Scripts and styles come from this origin only (no
    // inline), and a sink that takes a string as HTML or script (`innerHTML`, `eval`) is refused unless the
    // one policy of the page made it: Svelte's own, which wraps nothing a task's text could reach (the body
    // is drawn as elements). An image is of this origin, or a `blob:` the page made itself from an answer it read with
    // its session (an image route needs one, which an `<img src>` does not send); a `data:` image is not allowed.
    let policy = format!(
        "default-src 'none'; script-src 'self'; style-src 'self'; \
         img-src 'self' blob:; connect-src {connect};{install}{media} base-uri 'none'; \
         object-src 'none'; frame-ancestors 'none'; form-action 'none'; \
         require-trusted-types-for 'script'; trusted-types {trusted}"
    );
    // The origin and the policy name were checked when the rules were made (plain ASCII), so this cannot fail;
    // the fallback is the strictest policy.
    HeaderValue::from_str(&policy)
        .unwrap_or_else(|_| HeaderValue::from_static("default-src 'none'"))
}

pub fn set_security_headers(response: &mut Response, class: Class, connect_src: Option<&str>) {
    set_page_headers(response, class, connect_src, None, false);
}

/// The headers of every answer; `worker_policy` is set for a path of the installable page only, and `microphone`
/// only there, when the app let that page record (`with_installable_microphone`).
pub fn set_page_headers(
    response: &mut Response,
    class: Class,
    connect_src: Option<&str>,
    worker_policy: Option<&str>,
    microphone: bool,
) {
    let microphone = microphone && worker_policy.is_some();
    let h = response.headers_mut();
    h.insert(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(
        HeaderName::from_static("cross-origin-resource-policy"),
        HeaderValue::from_static("same-origin"),
    );
    if class == Class::Shell {
        // Nothing is allowed that the page does not name. Scripts and styles
        // come from this origin only (no inline), and a sink that takes a
        // string as HTML or script (`innerHTML`, `eval`) is refused unless
        // the one policy of the page made it: Svelte's own, which wraps
        // nothing a task's text could reach (the body is drawn as elements).
        h.insert(
            header::CONTENT_SECURITY_POLICY,
            page_policy(connect_src, worker_policy, microphone),
        );
        h.insert(
            HeaderName::from_static("cross-origin-opener-policy"),
            HeaderValue::from_static("same-origin"),
        );
        h.insert(
            HeaderName::from_static("permissions-policy"),
            HeaderValue::from_static(if microphone {
                "camera=(), microphone=(self), geolocation=(), payment=(), usb=(), \
                 clipboard-read=(), clipboard-write=(self)"
            } else {
                "camera=(), microphone=(), geolocation=(), payment=(), usb=(), \
                 clipboard-read=(), clipboard-write=(self)"
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use crate::*;

    fn connect_of(policy: &HeaderValue) -> String {
        let text = policy.to_str().unwrap();
        text.split(';')
            .map(str::trim)
            .find_map(|d| d.strip_prefix("connect-src "))
            .unwrap()
            .to_owned()
    }

    #[test]
    fn the_page_connects_to_this_origin_only_unless_the_app_names_one_more() {
        assert_eq!(connect_of(&super::shell_policy(None)), "'self'");
        assert_eq!(
            connect_of(&super::shell_policy(Some("https://api.github.com"))),
            "'self' https://api.github.com"
        );
        // Everything else of the policy is the same either way.
        let plain = super::shell_policy(None);
        let wide = super::shell_policy(Some("https://api.github.com"));
        assert_eq!(
            plain.to_str().unwrap().replace("connect-src 'self'", ""),
            wide.to_str()
                .unwrap()
                .replace("connect-src 'self' https://api.github.com", "")
        );
        assert_eq!(rules().connect_src(), None);
        assert_eq!(
            rules()
                .with_connect_src("https://api.github.com")
                .connect_src(),
            Some("https://api.github.com")
        );
    }

    #[test]
    fn only_a_plain_https_host_may_be_named_for_connect_src() {
        for good in [
            "https://api.github.com",
            "https://example.org:8443",
            "https://a-b.c1.example.io",
        ] {
            assert!(super::plain_https_origin(good), "{good}");
        }
        for bad in [
            "",
            "https://",
            "http://api.github.com",
            "https://*.github.com",
            "*",
            "https://api.github.com/",
            "https://api.github.com/x",
            "https://api.github.com?x=1",
            "https://api.github.com#x",
            "https://user@api.github.com",
            "https://api.github.com:0",
            "https://api.github.com:65536",
            "https://api.github.com:",
            "https://api.github.com:80a",
            "https://localhost",
            "https://127.0.0.1",
            "https://1.2.3.4:443",
            "https://[::1]",
            "https://API.github.com",
            "https://api..github.com",
            "https://-api.github.com",
            "https://api.github.com;",
            "https://api.github.com 'unsafe-eval'",
            "https://api.github.com\r\nX: y",
            "data:",
            "'self'",
        ] {
            assert!(!super::plain_https_origin(bad), "{bad:?}");
            assert!(
                std::panic::catch_unwind(|| rules().with_connect_src(bad)).is_err(),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn an_installable_page_takes_one_plain_prefix_and_one_plain_policy_name() {
        let r = rules().with_installable_page("/m", "tasks-mobile-worker");
        assert_eq!(r.installable_prefix(), Some("/m"));
        for yes in ["/m/", "/m/index.html", "/m/sw.js", "/m/assets/a.js"] {
            assert!(r.is_installable(yes), "{yes}");
            assert!(matches!(classify(yes, &r), Class::Shell), "{yes}");
        }
        for no in ["/m", "/mx/", "/x/m/", "/", "/api/m/"] {
            assert!(!r.is_installable(no), "{no}");
        }
        assert_eq!(
            r.installable_for("/m/sw.js")
                .map(|i| i.worker_policy.as_str()),
            Some("tasks-mobile-worker")
        );
        assert!(r.installable_for("/").is_none());
        // Without the option nothing is installable.
        assert!(!rules().is_installable("/m/"));
        assert!(matches!(classify("/m/", &rules()), Class::Other));

        for bad in [
            "m",
            "/",
            "/M",
            "/m/",
            "/m/x",
            "/a b",
            "/api",
            "/assets",
            "/auth",
            "/-m",
            "/m;",
            "/0123456789012345678901234567890123",
        ] {
            assert!(
                std::panic::catch_unwind(|| rules().with_installable_page(bad, "w")).is_err(),
                "{bad:?}"
            );
        }
        for bad in [
            "",
            "svelte-trusted-html",
            "default",
            "none",
            "Worker",
            "1w",
            "w x",
            "w;",
            "'none'",
            "w*",
        ] {
            assert!(
                std::panic::catch_unwind(|| rules().with_installable_page("/m", bad)).is_err(),
                "{bad:?}"
            );
        }
        assert!(std::panic::catch_unwind(|| rules()
            .with_installable_page("/m", "w")
            .with_installable_page("/n", "v"))
        .is_err());
    }

    #[test]
    fn an_image_is_of_this_origin_or_a_blob_of_the_page_and_never_data() {
        let policy = super::shell_policy(None);
        let img = policy
            .to_str()
            .unwrap()
            .split(';')
            .map(str::trim)
            .find_map(|d| d.strip_prefix("img-src "))
            .unwrap()
            .to_owned();
        assert_eq!(img, "'self' blob:");
    }

    #[test]
    fn the_installable_policy_adds_three_words_and_nothing_else() {
        let plain = super::page_policy(None, None, false);
        let page = super::page_policy(None, Some("w1"), false);
        assert_eq!(
            page.to_str()
                .unwrap()
                .replace(" manifest-src 'self'; worker-src 'self';", "")
                .replace("svelte-trusted-html w1", "svelte-trusted-html"),
            plain.to_str().unwrap()
        );
        assert_eq!(plain, super::shell_policy(None));
    }

    #[test]
    fn a_recording_installable_page_adds_media_from_blobs_and_only_there() {
        let page = super::page_policy(None, Some("w1"), false);
        let media = super::page_policy(None, Some("w1"), true);
        assert_eq!(
            media
                .to_str()
                .unwrap()
                .replace(" media-src 'self' blob:;", ""),
            page.to_str().unwrap()
        );
        assert!(media.to_str().unwrap().contains("media-src 'self' blob:;"));
        // Without the installable page there is nothing to record on: the shell keeps no media.
        assert_eq!(
            super::page_policy(None, None, true),
            super::page_policy(None, None, false)
        );
    }

    fn permissions_of(worker_policy: Option<&str>, microphone: bool) -> String {
        let mut response = axum::response::Response::new(axum::body::Body::empty());
        super::set_page_headers(&mut response, Class::Shell, None, worker_policy, microphone);
        response.headers()["permissions-policy"]
            .to_str()
            .unwrap()
            .to_owned()
    }

    #[test]
    fn the_microphone_is_off_unless_the_installable_page_was_let_record() {
        assert!(permissions_of(None, false).contains("microphone=()"));
        assert!(permissions_of(Some("w1"), false).contains("microphone=()"));
        assert!(permissions_of(None, true).contains("microphone=()"));
        let on = permissions_of(Some("w1"), true);
        assert!(on.contains("microphone=(self)"));
        assert!(on.contains("camera=()"));
    }

    #[test]
    #[should_panic(expected = "with_installable_page first")]
    fn the_microphone_needs_an_installable_page() {
        let _ = rules().with_installable_microphone();
    }

    fn rules() -> GuardRules {
        GuardRules::new(
            "/api/rules/hello",
            [
                "/api/rules/apply".to_owned(),
                "/api/rules/preview".to_owned(),
            ],
        )
    }

    #[test]
    fn only_the_hello_of_the_rules_is_open() {
        let r = rules();
        assert!(matches!(classify("/api/rules/hello", &r), Class::Hello));
        // The hello of another app is one more route that needs a session.
        assert!(matches!(classify("/api/tasks/hello", &r), Class::Api));
    }

    #[test]
    fn what_an_app_names_cannot_open_a_route_outside_api_or_take_one_of_the_core() {
        // The paths an app names are checked when the rules are made: each of these would open a route to
        // anyone (hello needs no session) or shadow a route of the core.
        let bad_hello = [
            "/hello",
            "/",
            "/index.html",
            "/api/",
            "/api",
            "/api/x/",
            "/api//x",
            "/api/../x",
            "/api/x?y=1",
            "/api/x y",
            "/api/control/pair",
            "/api/vaults",
            "/api/vaults/add",
            "/api/vaults/remove",
            "/api/vaults/select",
            "/api/vaults/update",
            "/auth/exchange",
        ];
        for path in bad_hello {
            let made = std::panic::catch_unwind(|| GuardRules::new(path, []));
            assert!(made.is_err(), "hello at {path:?} was accepted");
        }
        for path in ["/ops", "/api/ops/", "/api/../ops", "/assets/x"] {
            let made =
                std::panic::catch_unwind(|| GuardRules::new("/api/ok/hello", [path.to_owned()]));
            assert!(made.is_err(), "coarse path {path:?} was accepted");
        }
        // What the classes make of the paths outside /api/ does not depend on the rules.
        let r = rules();
        assert!(matches!(classify("/ops", &r), Class::Other));
        assert!(matches!(classify("/auth/exchange", &r), Class::Exchange));
        assert!(matches!(
            classify("/api/control/pair", &r),
            Class::ControlPair
        ));
        assert!(matches!(classify("/assets/app.js", &r), Class::Shell));
    }

    #[test]
    fn plain_routes_below_api_are_accepted() {
        let r = GuardRules::new(
            "/api/my-app_2/hello.v1",
            ["/api/my-app_2/plan_for".to_owned()],
        );
        assert_eq!(r.hello_path(), "/api/my-app_2/hello.v1");
        assert!(r.is_coarse("/api/my-app_2/plan_for"));
    }

    #[test]
    fn an_upload_path_is_a_route_the_app_named() {
        let rules = GuardRules::new("/api/x/hello", Vec::new()).with_upload_path("/api/x/upload");
        assert!(rules.is_upload("/api/x/upload"));
        assert!(!rules.is_upload("/api/x/upload/"));
        assert!(!rules.is_upload("/api/x/other"));
        assert!(!GuardRules::new("/api/x/hello", Vec::new()).is_upload("/api/x/upload"));
    }

    #[test]
    #[should_panic(expected = "route of the app")]
    fn the_pairing_route_is_no_upload_path() {
        let _ = GuardRules::new("/api/x/hello", Vec::new()).with_upload_path("/api/control/pair");
    }

    #[test]
    #[should_panic(expected = "plain route")]
    fn an_upload_path_is_below_the_api() {
        let _ = GuardRules::new("/api/x/hello", Vec::new()).with_upload_path("/up");
    }

    #[test]
    fn the_coarse_limit_is_for_the_listed_paths_only() {
        let r = rules();
        assert!(r.is_coarse("/api/rules/apply"));
        assert!(r.is_coarse("/api/rules/preview"));
        assert!(!r.is_coarse("/api/rules/other"));
        assert!(!r.is_coarse("/api/rules/apply/"));
    }
}
