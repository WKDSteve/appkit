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
}

/// The routes of the core itself: an app cannot name one of them as its own.
const CORE_PATHS: [&str; 7] = [
    "/api/control/pair",
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
            connect_src: None,
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
        p if p == rules.hello_path => Class::Hello,
        "/auth/exchange" => Class::Exchange,
        "/api/control/pair" => Class::ControlPair,
        p if p.starts_with("/api/") => Class::Api,
        _ => Class::Other,
    }
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
    let mut response = judge(&st, class, req, next).await;
    set_security_headers(&mut response, class, st.guard_rules.connect_src());
    response
}

pub async fn judge<R: Runtime>(st: &Core<R>, class: Class, req: Request, next: Next) -> Response {
    let (parts, body) = req.into_parts();
    let headers = &parts.headers;

    // 1. Host
    match single(headers, &header::HOST) {
        Ok(Some(host)) if st.policy.host_ok(&host) => {}
        Ok(_) => return error_body(StatusCode::FORBIDDEN, "bad_host", "unknown host"),
        Err(()) => return error_body(StatusCode::BAD_REQUEST, "bad_header", "invalid Host header"),
    }

    let mutation = parts.method != Method::GET;

    // 2. Declared body size
    // (No header at all is fine: a GET, or a chunked body that the body
    // limit of the extractor bounds while it is read.)
    let body_limit = match class {
        Class::Exchange | Class::ControlPair => st.limits.small_body_bytes,
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
    if !fetch_site_ok(fetch_site.as_deref()) {
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

    // 7. The credential the route needs
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
            let ok = bearer(headers).is_some_and(|t| st.sessions.check(&t));
            if !ok {
                return error_body(StatusCode::UNAUTHORIZED, "unauthorized", "session required");
            }
        }
    }

    // 8. A mutation carries JSON
    if mutation && !is_json_content_type(headers) {
        return error_body(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "bad_content_type",
            "application/json required",
        );
    }

    let req = Request::from_parts(parts, body);
    match tokio::time::timeout(st.limits.request_timeout, next.run(req)).await {
        Ok(response) => response,
        Err(_) => error_body(
            StatusCode::REQUEST_TIMEOUT,
            "timeout",
            "the request took too long",
        ),
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
fn shell_policy(connect_src: Option<&str>) -> HeaderValue {
    let connect = match connect_src {
        Some(origin) => format!("'self' {origin}"),
        None => "'self'".to_owned(),
    };
    // Nothing is allowed that the page does not name. Scripts and styles come from this origin only (no
    // inline), and a sink that takes a string as HTML or script (`innerHTML`, `eval`) is refused unless the
    // one policy of the page made it: Svelte's own, which wraps nothing a task's text could reach (the body
    // is drawn as elements).
    let policy = format!(
        "default-src 'none'; script-src 'self'; style-src 'self'; \
         img-src 'self'; connect-src {connect}; base-uri 'none'; \
         object-src 'none'; frame-ancestors 'none'; form-action 'none'; \
         require-trusted-types-for 'script'; trusted-types svelte-trusted-html"
    );
    // The origin was checked when the rules were made (plain ASCII), so this cannot fail; the fallback is
    // the strictest policy.
    HeaderValue::from_str(&policy)
        .unwrap_or_else(|_| HeaderValue::from_static("default-src 'none'"))
}

pub fn set_security_headers(response: &mut Response, class: Class, connect_src: Option<&str>) {
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
        h.insert(header::CONTENT_SECURITY_POLICY, shell_policy(connect_src));
        h.insert(
            HeaderName::from_static("cross-origin-opener-policy"),
            HeaderValue::from_static("same-origin"),
        );
        h.insert(
            HeaderName::from_static("permissions-policy"),
            HeaderValue::from_static(
                "camera=(), microphone=(), geolocation=(), payment=(), usb=(), \
                 clipboard-read=(), clipboard-write=(self)",
            ),
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
    fn the_coarse_limit_is_for_the_listed_paths_only() {
        let r = rules();
        assert!(r.is_coarse("/api/rules/apply"));
        assert!(r.is_coarse("/api/rules/preview"));
        assert!(!r.is_coarse("/api/rules/other"));
        assert!(!r.is_coarse("/api/rules/apply/"));
    }
}
