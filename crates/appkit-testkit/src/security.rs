//! The security tests of the guard (concept 5.5), written once and run for every app.
//!
//! Requests are written as raw bytes over a TCP socket: a real HTTP client would "repair" the very
//! things under test (the `Host` header, a header that is too long, a content type). Each test
//! runs against a real server bound to port 0.
//!
//! A test is a function of a [`Subject`], the app whose hub is under test; the macro
//! [`security_matrix!`](crate::security_matrix) makes a `#[tokio::test]` of each. The paths come
//! from the identity of the subject, never from a literal: what the matrix asserts about the guard
//! must hold under any namespace.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use appkit_core::authn::{ManualClock, Secret, CODE_BYTES, TOKEN_BYTES};
use appkit_core::identity::AppIdentity;
use appkit_hub::{HubStatus, Limits, RunningServer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::subject::{StartConfig, Subject};
use crate::wire::{request, send, Reply};

/// A method, a path and the extra headers of one request.
type Attempt<'a> = (&'a str, &'a str, Vec<(&'a str, &'a str)>);

// ── harness ─────────────────────────────────────────────────────────────────

struct Harness {
    server: RunningServer,
    clock: Arc<ManualClock>,
    control: String,
    identity: &'static AppIdentity,
}

impl Harness {
    async fn new(s: &dyn Subject) -> Self {
        Self::with(s, Limits::default()).await
    }

    async fn with(s: &dyn Subject, limits: Limits) -> Self {
        let clock = Arc::new(ManualClock::new());
        let control = Secret::generate(TOKEN_BYTES).unwrap();
        let server = s
            .start(StartConfig {
                addr: "127.0.0.1:0".parse().unwrap(),
                limits,
                clock: clock.clone(),
                pairing_ttl: Duration::from_secs(60),
                control_secret: control.clone(),
                instance: Secret::generate(CODE_BYTES).unwrap(),
                status: HubStatus {
                    engine_ready: true,
                    problem: None,
                    capabilities: Vec::new(),
                },
            })
            .await
            .expect("the server starts");
        Self {
            server,
            clock,
            control: control.to_hex(),
            identity: s.identity(),
        }
    }

    /// The path of a route of the app.
    fn route(&self, name: &str) -> String {
        self.identity.route(name)
    }

    fn addr(&self) -> SocketAddr {
        self.server.addr()
    }

    fn port(&self) -> u16 {
        self.server.addr().port()
    }

    fn host(&self) -> String {
        format!("127.0.0.1:{}", self.port())
    }

    fn origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.port())
    }
}

/// A GET with a good `Host` and the extra headers given.
async fn get(h: &Harness, path: &str, extra: &[(&str, &str)]) -> Reply {
    let host = h.host();
    let mut headers = vec![("Host", host.as_str())];
    headers.extend_from_slice(extra);
    send(h.addr(), request("GET", path, &headers, b"")).await
}

/// A POST of JSON text with a good `Host`, and the extra headers given.
async fn post(h: &Harness, path: &str, extra: &[(&str, &str)], body: &str) -> Reply {
    let host = h.host();
    let len = body.len().to_string();
    let mut headers = vec![
        ("Host", host.as_str()),
        ("Content-Type", "application/json"),
        ("Content-Length", len.as_str()),
    ];
    headers.extend_from_slice(extra);
    send(h.addr(), request("POST", path, &headers, body.as_bytes())).await
}

fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

/// Ask for a pairing code the way `<program> open` does.
async fn mint_code(h: &Harness) -> String {
    let auth = bearer(&h.control);
    let r = post(h, "/api/control/pair", &[("Authorization", &auth)], "{}").await;
    assert_eq!(r.status, 200, "{r:?}");
    r.json()["code"].as_str().expect("a code").to_owned()
}

/// Exchange a code the way the browser does: with its Origin.
async fn exchange(h: &Harness, code: &str) -> Reply {
    let origin = h.origin();
    let body = format!(r#"{{"code":"{code}"}}"#);
    post(h, "/auth/exchange", &[("Origin", &origin)], &body).await
}

async fn session(h: &Harness) -> String {
    let code = mint_code(h).await;
    let r = exchange(h, &code).await;
    assert_eq!(r.status, 200, "{r:?}");
    r.json()["token"].as_str().expect("a token").to_owned()
}

// ── 1, 2: Host ──────────────────────────────────────────────────────────────

pub async fn a_wrong_host_is_forbidden(s: &dyn Subject) {
    let h = Harness::new(s).await;
    let port = h.port();
    for bad in [
        "evil.example".to_owned(),
        "127.0.0.1".to_owned(),
        "127.0.0.1:1".to_owned(),
        "0x7f.1".to_owned(),
        format!("127.0.0.1:{port}.evil.example"),
        format!("evil.example:{port}"),
        format!("LOCALHOST:{port}"),
        format!("127.0.0.1:{port}@evil.example"),
    ] {
        let r = send(
            h.addr(),
            request("GET", h.route("hello").as_str(), &[("Host", &bad)], b""),
        )
        .await;
        assert_eq!(r.status, 403, "Host {bad:?} was accepted: {r:?}");
    }
}

pub async fn the_loopback_names_are_accepted(s: &dyn Subject) {
    let h = Harness::new(s).await;
    let port = h.port();
    for good in [
        format!("127.0.0.1:{port}"),
        format!("localhost:{port}"),
        format!("[::1]:{port}"),
    ] {
        let r = send(
            h.addr(),
            request("GET", h.route("hello").as_str(), &[("Host", &good)], b""),
        )
        .await;
        assert_eq!(r.status, 200, "Host {good:?} was refused: {r:?}");
    }
}

pub async fn two_host_headers_are_a_bad_request(s: &dyn Subject) {
    let h = Harness::new(s).await;
    let host = h.host();
    let r = send(
        h.addr(),
        request(
            "GET",
            h.route("hello").as_str(),
            &[("Host", &host), ("Host", "evil.example")],
            b"",
        ),
    )
    .await;
    assert!(matches!(r.status, 400 | 403), "{r:?}");
}

// ── 3, 5: Origin and Fetch Metadata ─────────────────────────────────────────

pub async fn a_mutation_needs_a_good_origin(s: &dyn Subject) {
    let h = Harness::new(s).await;
    let code = mint_code(&h).await;
    let body = format!(r#"{{"code":"{code}"}}"#);
    // Wrong, "null", and a look-alike: refused. The code stays unspent.
    for bad in [
        "null".to_owned(),
        "http://evil.example".to_owned(),
        format!("{}.evil.example", h.origin()),
        format!("{}/", h.origin()),
        "https://127.0.0.1".to_owned(),
    ] {
        let r = post(&h, "/auth/exchange", &[("Origin", &bad)], &body).await;
        assert_eq!(r.status, 403, "Origin {bad:?} was accepted: {r:?}");
    }
    // Missing: a browser always sends one with a POST, so this is refused too.
    let r = post(&h, "/auth/exchange", &[], &body).await;
    assert_eq!(r.status, 403, "{r:?}");
    // The code was never spent by the refused tries.
    assert_eq!(exchange(&h, &code).await.status, 200);
}

pub async fn cross_site_requests_are_forbidden(s: &dyn Subject) {
    let h = Harness::new(s).await;
    let token = session(&h).await;
    let auth = bearer(&token);
    for site in ["cross-site", "same-site", ""] {
        let r = get(&h, h.route("hello").as_str(), &[("Sec-Fetch-Site", site)]).await;
        assert_eq!(r.status, 403, "Sec-Fetch-Site {site:?}: {r:?}");
        let r = get(
            &h,
            h.route("anything").as_str(),
            &[("Sec-Fetch-Site", site), ("Authorization", &auth)],
        )
        .await;
        assert_eq!(r.status, 403, "Sec-Fetch-Site {site:?} with token: {r:?}");
    }
}

pub async fn a_same_origin_get_without_origin_header_gets_through(s: &dyn Subject) {
    // What an EventSource-by-fetch looks like in a browser: a token, a
    // `Sec-Fetch-Site` of the same origin, and no `Origin` header at all.
    let h = Harness::new(s).await;
    let token = session(&h).await;
    let auth = bearer(&token);
    for site in [Some("same-origin"), Some("none"), None] {
        let mut extra = vec![("Authorization", auth.as_str())];
        if let Some(s) = site {
            extra.push(("Sec-Fetch-Site", s));
        }
        let r = get(&h, h.route("not-built-yet").as_str(), &extra).await;
        // Past the guard, so the route says "not found", not "forbidden".
        assert_eq!(r.status, 404, "site {site:?}: {r:?}");
    }
}

pub async fn a_browser_mutation_without_origin_is_refused_but_a_program_may(s: &dyn Subject) {
    let h = Harness::new(s).await;
    let token = session(&h).await;
    let auth = bearer(&token);
    // A program client: a token, no Origin, no Fetch Metadata.
    let r = post(
        &h,
        h.route("not-built-yet").as_str(),
        &[("Authorization", &auth)],
        "{}",
    )
    .await;
    assert_eq!(r.status, 404, "{r:?}");
    // Looks like a browser (it sends Fetch Metadata) but has no Origin.
    let r = post(
        &h,
        h.route("not-built-yet").as_str(),
        &[("Authorization", &auth), ("Sec-Fetch-Site", "same-origin")],
        "{}",
    )
    .await;
    assert_eq!(r.status, 403, "{r:?}");
}

pub async fn there_is_no_cors(s: &dyn Subject) {
    let h = Harness::new(s).await;
    // A preflight from another origin, and every method that is not ours.
    let host = h.host();
    let r = send(
        h.addr(),
        request(
            "OPTIONS",
            h.route("hello").as_str(),
            &[
                ("Host", &host),
                ("Origin", "http://evil.example"),
                ("Access-Control-Request-Method", "POST"),
            ],
            b"",
        ),
    )
    .await;
    assert_eq!(r.status, 403, "{r:?}");
    assert!(r.header("access-control-allow-origin").is_none());
    for method in ["PUT", "DELETE", "PATCH", "OPTIONS", "HEAD", "TRACE"] {
        let r = send(
            h.addr(),
            request(method, h.route("hello").as_str(), &[("Host", &host)], b""),
        )
        .await;
        assert_eq!(r.status, 405, "{method}: {r:?}");
        assert!(r.header("access-control-allow-origin").is_none());
    }
    let r = get(&h, h.route("hello").as_str(), &[("Origin", &h.origin())]).await;
    assert!(r.header("access-control-allow-origin").is_none(), "{r:?}");
}

// ── 4: credentials ──────────────────────────────────────────────────────────

pub async fn every_data_route_needs_a_session(s: &dyn Subject) {
    let h = Harness::new(s).await;
    let token = session(&h).await;
    let control = h.control.clone();
    let wrong_token = Secret::generate(TOKEN_BYTES).unwrap().to_hex();
    // The routes the app names, and a few that are not any route: the guard decides by the
    // path alone, before a route is looked for.
    let mut paths: Vec<String> = s.data_routes().iter().map(|n| h.route(n)).collect();
    assert!(
        !paths.is_empty(),
        "a subject names the routes that need a session"
    );
    paths.extend([
        "/api/anything-at-all".to_owned(),
        format!("{}/", h.route("hello")),
        h.route("%68ello"),
        "/api/control/pair/".to_owned(),
    ]);
    for path in &paths {
        // No credential, and credentials of the wrong shape or kind.
        let attempts: Vec<Vec<(&str, String)>> = vec![
            vec![],
            vec![("Authorization", String::new())],
            vec![("Authorization", "Bearer".to_owned())],
            vec![("Authorization", "Bearer ".to_owned())],
            vec![("Authorization", format!("Basic {token}"))],
            vec![("Authorization", format!("bearer {token}"))],
            vec![("Authorization", token.clone())],
            vec![("Authorization", bearer(&token[..62]))],
            vec![("Authorization", bearer(&token.to_uppercase()))],
            vec![("Authorization", bearer(&wrong_token))],
            // The control secret is no session.
            vec![("Authorization", bearer(&control))],
        ];
        for extra in attempts {
            let extra: Vec<(&str, &str)> = extra.iter().map(|(k, v)| (*k, v.as_str())).collect();
            let r = get(&h, path, &extra).await;
            assert_eq!(r.status, 401, "{path} with {extra:?}: {r:?}");
        }
    }
}

pub async fn the_control_secret_and_a_session_are_not_interchangeable(s: &dyn Subject) {
    let h = Harness::new(s).await;
    let token = session(&h).await;
    // A session cannot mint a pairing code ...
    let auth = bearer(&token);
    let r = post(&h, "/api/control/pair", &[("Authorization", &auth)], "{}").await;
    assert_eq!(r.status, 401, "{r:?}");
    // ... and the control secret is no session.
    let auth = bearer(&h.control);
    let r = get(
        &h,
        h.route("not-built-yet").as_str(),
        &[("Authorization", &auth)],
    )
    .await;
    assert_eq!(r.status, 401, "{r:?}");
    // No credential at all cannot mint a code either.
    let r = post(&h, "/api/control/pair", &[], "{}").await;
    assert_eq!(r.status, 401, "{r:?}");
}

pub async fn a_code_is_not_a_token(s: &dyn Subject) {
    let h = Harness::new(s).await;
    let code = mint_code(&h).await;
    let auth = bearer(&code);
    let r = get(
        &h,
        h.route("not-built-yet").as_str(),
        &[("Authorization", &auth)],
    )
    .await;
    assert_eq!(r.status, 401, "{r:?}");
}

// ── 6, 7, 8: content type and size ──────────────────────────────────────────

pub async fn a_mutation_must_be_json(s: &dyn Subject) {
    let h = Harness::new(s).await;
    let token = session(&h).await;
    let auth = bearer(&token);
    let origin = h.origin();
    let host = h.host();
    for ct in [
        "text/plain",
        "application/x-www-form-urlencoded",
        "multipart/form-data; boundary=x",
        "application/json-patch+json",
        "text/plain; application/json",
    ] {
        for (path, extra) in [
            ("/auth/exchange", vec![("Origin", origin.as_str())]),
            (
                h.route("not-built-yet").as_str(),
                vec![("Authorization", auth.as_str())],
            ),
        ] {
            let mut headers = vec![
                ("Host", host.as_str()),
                ("Content-Type", ct),
                ("Content-Length", "2"),
            ];
            headers.extend(extra);
            let r = send(h.addr(), request("POST", path, &headers, b"{}")).await;
            assert_eq!(r.status, 415, "{path} with {ct:?}: {r:?}");
        }
    }
    // No content type at all.
    let r = send(
        h.addr(),
        request(
            "POST",
            "/auth/exchange",
            &[
                ("Host", &host),
                ("Origin", &origin),
                ("Content-Length", "2"),
            ],
            b"{}",
        ),
    )
    .await;
    assert_eq!(r.status, 415, "{r:?}");
    // The proper forms do pass the guard.
    for ct in [
        "application/json",
        "application/json; charset=utf-8",
        "Application/JSON",
    ] {
        let headers = [
            ("Host", host.as_str()),
            ("Origin", origin.as_str()),
            ("Content-Type", ct),
            ("Content-Length", "2"),
        ];
        let r = send(h.addr(), request("POST", "/auth/exchange", &headers, b"{}")).await;
        // Past the guard: the body is missing its `code`, so 400.
        assert_eq!(r.status, 400, "{ct:?}: {r:?}");
    }
}

pub async fn a_body_over_the_limit_is_refused_unread(s: &dyn Subject) {
    let h = Harness::new(s).await;
    let host = h.host();
    let origin = h.origin();
    // 2 MiB declared; only the head is sent, the server must not wait for it.
    let r = send(
        h.addr(),
        request(
            "POST",
            "/auth/exchange",
            &[
                ("Host", &host),
                ("Origin", &origin),
                ("Content-Type", "application/json"),
                ("Content-Length", "2097152"),
            ],
            b"",
        ),
    )
    .await;
    assert_eq!(r.status, 413, "{r:?}");
    // Exactly at the limit is not refused by the size rule.
    let r = send(
        h.addr(),
        request(
            "POST",
            "/auth/exchange",
            &[
                ("Host", &host),
                ("Origin", &origin),
                ("Content-Type", "application/json"),
                ("Content-Length", "-1"),
            ],
            b"",
        ),
    )
    .await;
    assert_eq!(r.status, 400, "{r:?}");
}

pub async fn a_chunked_body_over_the_limit_is_refused_while_it_is_read(s: &dyn Subject) {
    let limits = Limits {
        max_body_bytes: 64,
        small_body_bytes: 64,
        ..Limits::default()
    };
    let h = Harness::with(s, limits).await;
    let host = h.host();
    let origin = h.origin();
    let mut body = Vec::new();
    for _ in 0..4 {
        body.extend_from_slice(b"32\r\n");
        body.extend_from_slice(&[b'a'; 0x32]);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(b"0\r\n\r\n");
    let r = send(
        h.addr(),
        request(
            "POST",
            "/auth/exchange",
            &[
                ("Host", &host),
                ("Origin", &origin),
                ("Content-Type", "application/json"),
                ("Transfer-Encoding", "chunked"),
            ],
            &body,
        ),
    )
    .await;
    assert_eq!(r.status, 413, "{r:?}");
}

pub async fn an_overlong_request_line_or_header_is_refused_and_the_server_lives(s: &dyn Subject) {
    let h = Harness::new(s).await;
    let host = h.host();
    let long_path = format!("/{}", "a".repeat(10 * 1024));
    let r = send(
        h.addr(),
        request("GET", &long_path, &[("Host", &host)], b""),
    )
    .await;
    assert!(matches!(r.status, 400 | 414 | 431), "long path: {r:?}");
    let long_value = "b".repeat(10 * 1024);
    let r = send(
        h.addr(),
        request(
            "GET",
            h.route("hello").as_str(),
            &[("Host", &host), ("X-Long", &long_value)],
            b"",
        ),
    )
    .await;
    assert!(matches!(r.status, 400 | 431), "long header: {r:?}");
    // Still answering.
    assert_eq!(get(&h, h.route("hello").as_str(), &[]).await.status, 200);
}

// ── 9, 10: what needs no credential ─────────────────────────────────────────

pub async fn hello_needs_no_token_and_carries_no_data(s: &dyn Subject) {
    let h = Harness::new(s).await;
    let r = get(&h, h.route("hello").as_str(), &[]).await;
    assert_eq!(r.status, 200, "{r:?}");
    let doc = r.json();
    let mut keys: Vec<&str> = doc
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "app",
            "build",
            "capabilities",
            "engine",
            "instance",
            "kind",
            "schema",
            "setup_reason",
            "setup_required",
            "version"
        ]
    );
    // A hub that is set up says so with a boolean and no reason; a reason is a short code.
    assert!(doc["setup_required"].is_boolean(), "{}", r.body);
    match doc["setup_reason"].as_str() {
        None => assert!(doc["setup_reason"].is_null() && doc["setup_required"] == false),
        Some(code) => {
            assert_eq!(doc["setup_required"], true);
            assert!(
                !code.is_empty()
                    && code.len() <= 32
                    && code.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'),
                "setup_reason is not a short code: {code:?}"
            );
        }
    }
    // The build is the short commit id of a rolling build, or null: never a path or a sentence.
    if let Some(build) = doc["build"].as_str() {
        assert!(
            !build.is_empty()
                && build.len() <= 40
                && build.bytes().all(|b| b.is_ascii_alphanumeric()),
            "build is not a commit id: {build:?}"
        );
    } else {
        assert!(doc["build"].is_null(), "{}", r.body);
    }
    assert_eq!(doc["app"], s.identity().app);
    assert_eq!(doc["instance"], h.server.instance());
    // No path, no drive letter, no vault, no area.
    for needle in ["\\", ":/", "vault", "area", "ROADMAP", "Users"] {
        assert!(
            !r.body.contains(needle),
            "hello holds {needle:?}: {}",
            r.body
        );
    }
}

pub async fn the_page_needs_no_token_and_has_its_headers(s: &dyn Subject) {
    let h = Harness::new(s).await;
    let r = get(&h, "/", &[]).await;
    assert_eq!(r.status, 200, "{r:?}");
    assert!(r.header("content-type").unwrap().starts_with("text/html"));
    assert_eq!(r.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(r.header("referrer-policy"), Some("no-referrer"));
    assert_eq!(r.header("cache-control"), Some("no-store"));
    assert!(r
        .header("content-security-policy")
        .unwrap()
        .contains("frame-ancestors 'none'"));
    // The page reaches this origin and, at most, one plain https host the app named: no wildcard, no scheme
    // but https, no second host.
    let csp = r.header("content-security-policy").unwrap();
    let connect = csp
        .split(';')
        .map(str::trim)
        .find_map(|d| d.strip_prefix("connect-src "))
        .expect("the policy has a connect-src");
    let mut sources = connect.split(' ');
    assert_eq!(sources.next(), Some("'self'"), "{connect}");
    if let Some(extra) = sources.next() {
        assert!(
            extra.starts_with("https://") && !extra.contains('*') && !extra[8..].contains('/'),
            "{connect}"
        );
    }
    assert_eq!(sources.next(), None, "{connect}");
    // The page may write the clipboard (on a gesture), never read it, and no frame may do either.
    let policy = r.header("permissions-policy").unwrap();
    assert!(policy.contains("clipboard-write=(self)"), "{policy}");
    assert!(policy.contains("clipboard-read=()"), "{policy}");
    // Anything else outside /api is not found, with no token needed.
    let r = get(&h, "/etc/passwd", &[]).await;
    assert_eq!(r.status, 404, "{r:?}");
}

/// The page of the app may fetch from this origin and from `extra` (an `https` host the app named), and from
/// nothing else.
pub async fn the_page_may_connect_to(s: &dyn Subject, extra: Option<&str>) {
    let h = Harness::new(s).await;
    let r = get(&h, "/", &[]).await;
    assert_eq!(r.status, 200, "{r:?}");
    let csp = r.header("content-security-policy").unwrap();
    let want = match extra {
        Some(origin) => format!("connect-src 'self' {origin};"),
        None => "connect-src 'self';".to_owned(),
    };
    assert!(csp.contains(&want), "{csp}");
}

/// The directives of a policy, by name.
fn directives(csp: &str) -> std::collections::HashMap<String, String> {
    csp.split(';')
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(|d| match d.split_once(' ') {
            Some((name, value)) => (name.to_owned(), value.to_owned()),
            None => (d.to_owned(), String::new()),
        })
        .collect()
}

/// An app with an installable page under `prefix` (`GuardRules::with_installable_page`) that serves `{prefix}/`:
/// that page and the files below it are shells with no token needed, `GET` only; their policy is the page's plus
/// `manifest-src 'self'`, `worker-src 'self'` and the worker policy next to Svelte's, and nothing else is wider;
/// the policy of `/` stays without them; `{prefix}` without a slash and a path below the prefix the app does not
/// serve are not found, and a path that climbs out of the prefix reaches no data route.
pub async fn the_installable_page_has_its_own_policy_and_nothing_more(
    s: &dyn Subject,
    prefix: &str,
    worker_policy: &str,
) {
    let h = Harness::new(s).await;
    let desk = get(&h, "/", &[]).await;
    let page = get(&h, &format!("{prefix}/"), &[]).await;
    assert_eq!(page.status, 200, "{page:?}");
    let desk_csp = directives(desk.header("content-security-policy").unwrap());
    let page_csp = directives(page.header("content-security-policy").unwrap());
    assert_eq!(
        page_csp.get("manifest-src").map(String::as_str),
        Some("'self'")
    );
    assert_eq!(
        page_csp.get("worker-src").map(String::as_str),
        Some("'self'")
    );
    assert_eq!(
        page_csp.get("trusted-types").map(String::as_str),
        Some(format!("svelte-trusted-html {worker_policy}").as_str())
    );
    // Everything else is the desktop page's, word for word.
    for (name, value) in &desk_csp {
        if name != "trusted-types" {
            assert_eq!(page_csp.get(name), Some(value), "{name} differs");
        }
    }
    assert_eq!(
        page_csp.len(),
        desk_csp.len() + 2,
        "only manifest-src and worker-src are added: {page_csp:?}"
    );
    assert!(!desk_csp.contains_key("manifest-src") && !desk_csp.contains_key("worker-src"));
    assert_eq!(
        desk_csp.get("trusted-types").map(String::as_str),
        Some("svelte-trusted-html")
    );
    for header in [
        "x-content-type-options",
        "referrer-policy",
        "cache-control",
        "permissions-policy",
    ] {
        assert_eq!(page.header(header), desk.header(header), "{header}");
    }
    // A file below the prefix carries the same policy.
    let worker = get(&h, &format!("{prefix}/sw.js"), &[]).await;
    assert_eq!(worker.status, 200, "{worker:?}");
    assert_eq!(
        worker.header("content-security-policy"),
        page.header("content-security-policy")
    );
    // No credential needed, no other method, nothing that is not there.
    for path in [
        prefix.to_owned(),
        format!("{prefix}/nope.js"),
        format!("{prefix}x/"),
    ] {
        let r = get(&h, &path, &[]).await;
        assert_eq!(r.status, 404, "{path}: {r:?}");
        assert!(
            !r.header("content-security-policy")
                .unwrap_or("")
                .contains("worker-src")
                || path.starts_with(&format!("{prefix}/")),
            "{path} has the installable policy"
        );
    }
    let r = post(
        &h,
        &format!("{prefix}/"),
        &[("Origin", h.origin().as_str())],
        "{}",
    )
    .await;
    assert_eq!(r.status, 405, "{r:?}");
    // Climbing out of the prefix is not a way to a data route without a session.
    let climb = format!("{prefix}/..{}", h.route(s.data_routes()[0]));
    let r = get(&h, &climb, &[]).await;
    assert_eq!(r.status, 404, "{climb}: {r:?}");
}

pub async fn every_answer_carries_the_security_headers(s: &dyn Subject) {
    let h = Harness::new(s).await;
    let host = h.host();
    let answers = vec![
        get(&h, h.route("hello").as_str(), &[]).await,
        get(&h, h.route("snapshot").as_str(), &[]).await,
        get(&h, "/nope", &[]).await,
        post(&h, "/auth/exchange", &[], "{}").await,
        send(
            h.addr(),
            request(
                "GET",
                h.route("hello").as_str(),
                &[("Host", "evil.example")],
                b"",
            ),
        )
        .await,
        send(h.addr(), request("TRACE", "/", &[("Host", &host)], b"")).await,
    ];
    for r in answers {
        assert_eq!(r.header("x-content-type-options"), Some("nosniff"), "{r:?}");
        assert_eq!(r.header("referrer-policy"), Some("no-referrer"), "{r:?}");
        assert_eq!(r.header("cache-control"), Some("no-store"), "{r:?}");
    }
}

// ── 11 to 14, 17: pairing ───────────────────────────────────────────────────

pub async fn a_pairing_code_is_good_once(s: &dyn Subject) {
    let h = Harness::new(s).await;
    let code = mint_code(&h).await;
    let first = exchange(&h, &code).await;
    assert_eq!(first.status, 200, "{first:?}");
    let token = first.json()["token"].as_str().unwrap().to_owned();
    assert_eq!(token.len(), 64);
    let second = exchange(&h, &code).await;
    assert_eq!(second.status, 401, "{second:?}");
    // The session works.
    let auth = bearer(&token);
    let r = get(
        &h,
        h.route("not-built-yet").as_str(),
        &[("Authorization", &auth)],
    )
    .await;
    assert_eq!(r.status, 404, "{r:?}");
}

pub async fn a_pairing_code_expires(s: &dyn Subject) {
    let h = Harness::new(s).await;
    let code = mint_code(&h).await;
    h.clock.advance(Duration::from_secs(61));
    let r = exchange(&h, &code).await;
    assert_eq!(r.status, 401, "{r:?}");
    // The answer for an expired code is the answer for an unknown one.
    let unknown = exchange(&h, &"0".repeat(32)).await;
    assert_eq!(r.body, unknown.body);
}

pub async fn a_pairing_code_cannot_be_got_without_the_control_secret(s: &dyn Subject) {
    let h = Harness::new(s).await;
    let origin = h.origin();
    let wrong = bearer(&Secret::generate(TOKEN_BYTES).unwrap().to_hex());
    let hello_pair = format!("{}?pair=1", h.route("hello"));
    let pair_in_namespace = h.route("pair");
    let attempts: Vec<Attempt> = vec![
        ("POST", "/api/control/pair", vec![]),
        (
            "POST",
            "/api/control/pair",
            vec![("Origin", origin.as_str())],
        ),
        (
            "POST",
            "/api/control/pair",
            vec![("Authorization", wrong.as_str())],
        ),
        ("GET", "/api/control/pair", vec![]),
        ("GET", "/auth/exchange", vec![]),
        ("POST", pair_in_namespace.as_str(), vec![]),
        ("POST", "/auth/pair", vec![]),
        ("GET", hello_pair.as_str(), vec![]),
    ];
    for (method, path, extra) in attempts {
        let host = h.host();
        let mut headers = vec![
            ("Host", host.as_str()),
            ("Content-Type", "application/json"),
            ("Content-Length", "2"),
        ];
        headers.extend(extra);
        let body: &[u8] = if method == "POST" { b"{}" } else { b"" };
        let r = send(h.addr(), request(method, path, &headers, body)).await;
        assert!(
            matches!(r.status, 200 | 401 | 403 | 404 | 405)
                && !has_code(&r.body.replace(h.server.instance(), "")),
            "{method} {path}: {r:?}"
        );
        if path != hello_pair {
            assert_ne!(r.status, 200, "{method} {path}: {r:?}");
        }
    }
}

fn has_code(text: &str) -> bool {
    // 32 lowercase hex characters in a row.
    let mut run = 0;
    for c in text.chars() {
        if c.is_ascii_digit() || ('a'..='f').contains(&c) {
            run += 1;
            if run >= 32 {
                return true;
            }
        } else {
            run = 0;
        }
    }
    false
}

pub async fn a_token_is_never_echoed(s: &dyn Subject) {
    let h = Harness::new(s).await;
    let code = mint_code(&h).await;
    let issued = exchange(&h, &code).await;
    let token = issued.json()["token"].as_str().unwrap().to_owned();
    let auth = bearer(&token);
    let mut answers = vec![
        get(&h, h.route("hello").as_str(), &[("Authorization", &auth)]).await,
        get(
            &h,
            h.route("not-built-yet").as_str(),
            &[("Authorization", &auth)],
        )
        .await,
        get(&h, "/", &[("Authorization", &auth)]).await,
        post(
            &h,
            h.route("not-built-yet").as_str(),
            &[("Authorization", &auth)],
            "{}",
        )
        .await,
        post(&h, "/auth/exchange", &[("Authorization", &auth)], "{}").await,
        // A bad request that has the token in it.
        get(&h, &h.route(&token), &[("Authorization", &auth)]).await,
    ];
    answers.push(
        send(
            h.addr(),
            request(
                "GET",
                h.route("hello").as_str(),
                &[("Host", "evil.example"), ("Authorization", &auth)],
                b"",
            ),
        )
        .await,
    );
    for r in answers {
        assert!(!r.raw.contains(&token), "the token came back: {r:?}");
        assert!(!r.raw.contains(&h.control), "the control secret came back");
    }
}

pub async fn exchange_attempts_are_rate_limited(s: &dyn Subject) {
    let limits = Limits {
        exchange_max: 3,
        ..Limits::default()
    };
    let h = Harness::with(s, limits).await;
    for _ in 0..3 {
        assert_eq!(exchange(&h, &"0".repeat(32)).await.status, 401);
    }
    assert_eq!(exchange(&h, &"0".repeat(32)).await.status, 429);
    // A good code does not get past the limit either.
    let code = mint_code(&h).await;
    assert_eq!(exchange(&h, &code).await.status, 429);
    // The window ends.
    h.clock.advance(Duration::from_secs(61));
    let fresh = mint_code(&h).await;
    assert_eq!(exchange(&h, &fresh).await.status, 200);
}

pub async fn mutations_are_rate_limited(s: &dyn Subject) {
    let limits = Limits {
        post_max: 3,
        ..Limits::default()
    };
    let h = Harness::with(s, limits).await;
    for _ in 0..3 {
        assert_eq!(exchange(&h, &"0".repeat(32)).await.status, 401);
    }
    assert_eq!(exchange(&h, &"0".repeat(32)).await.status, 429);
    // Reads are not counted.
    assert_eq!(get(&h, h.route("hello").as_str(), &[]).await.status, 200);
    h.clock.advance(Duration::from_secs(11));
    assert_eq!(exchange(&h, &"0".repeat(32)).await.status, 401);
}

// ── 18, 20: binding and connections ─────────────────────────────────────────

pub async fn the_server_binds_loopback_only(s: &dyn Subject) {
    let h = Harness::new(s).await;
    assert!(h.addr().ip().is_loopback(), "{}", h.addr());
    let cfg = |addr: &str| StartConfig {
        addr: addr.parse().unwrap(),
        limits: Limits::default(),
        clock: Arc::new(ManualClock::new()),
        pairing_ttl: Duration::from_secs(60),
        control_secret: Secret::generate(TOKEN_BYTES).unwrap(),
        instance: Secret::generate(CODE_BYTES).unwrap(),
        status: HubStatus {
            engine_ready: false,
            problem: Some("no_vault"),
            capabilities: Vec::new(),
        },
    };
    for bad in ["0.0.0.0:0", "[::]:0", "192.168.1.10:0", "10.0.0.1:0"] {
        let err = s.start(cfg(bad)).await.err().expect("must refuse");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput, "{bad}");
    }
}

pub async fn the_connection_limit_closes_the_extra_connection(s: &dyn Subject) {
    let limits = Limits {
        max_connections: 2,
        ..Limits::default()
    };
    let h = Harness::with(s, limits).await;
    let _a = TcpStream::connect(h.addr()).await.unwrap();
    let _b = TcpStream::connect(h.addr()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    let reply = send(
        h.addr(),
        request(
            "GET",
            h.route("hello").as_str(),
            &[("Host", &h.host())],
            b"",
        ),
    )
    .await;
    assert_eq!(
        reply.status, 0,
        "the third connection was served: {reply:?}"
    );
    // A slot is free again once one connection ends.
    drop(_a);
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(get(&h, h.route("hello").as_str(), &[]).await.status, 200);
}

pub async fn a_client_that_sends_nothing_is_dropped(s: &dyn Subject) {
    let limits = Limits {
        header_timeout: Duration::from_millis(300),
        ..Limits::default()
    };
    let h = Harness::with(s, limits).await;
    let mut idle = TcpStream::connect(h.addr()).await.unwrap();
    let mut buf = Vec::new();
    // Half a request line, then silence.
    let hello = h.route("hello");
    let half = format!("GET {}", &hello[..hello.len() - 2]);
    idle.write_all(half.as_bytes()).await.unwrap();
    let ended = tokio::time::timeout(Duration::from_secs(5), idle.read_to_end(&mut buf)).await;
    assert!(ended.is_ok(), "the slow client was not dropped");
}

pub async fn a_silent_connection_is_dropped(s: &dyn Subject) {
    // Not even half a request line: the slot must not be held for ever.
    let limits = Limits {
        header_timeout: Duration::from_millis(300),
        ..Limits::default()
    };
    let h = Harness::with(s, limits).await;
    let mut silent = TcpStream::connect(h.addr()).await.unwrap();
    let mut buf = Vec::new();
    let ended = tokio::time::timeout(Duration::from_secs(4), silent.read_to_end(&mut buf)).await;
    assert!(ended.is_ok(), "a connection that sent nothing was kept");
}

pub async fn an_idle_keep_alive_connection_is_dropped(s: &dyn Subject) {
    let limits = Limits {
        header_timeout: Duration::from_millis(300),
        ..Limits::default()
    };
    let h = Harness::with(s, limits).await;
    let mut conn = TcpStream::connect(h.addr()).await.unwrap();
    let head = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\n\r\n",
        h.route("hello"),
        h.host()
    );
    conn.write_all(head.as_bytes()).await.unwrap();
    // Read the first answer (a short JSON body), then say nothing more.
    let mut seen = Vec::new();
    let mut chunk = [0u8; 1024];
    let first = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let n = conn.read(&mut chunk).await.unwrap();
            assert!(n > 0, "closed before the answer");
            seen.extend_from_slice(&chunk[..n]);
            if String::from_utf8_lossy(&seen).contains("\"capabilities\"") {
                break;
            }
        }
    })
    .await;
    assert!(first.is_ok(), "no answer");
    let mut rest = Vec::new();
    let ended = tokio::time::timeout(Duration::from_secs(4), conn.read_to_end(&mut rest)).await;
    assert!(ended.is_ok(), "an idle keep-alive connection was kept");
}

pub async fn the_routes_that_take_a_few_bytes_take_only_a_few_bytes(s: &dyn Subject) {
    let h = Harness::new(s).await;
    let host = h.host();
    let origin = h.origin();
    // Declared: refused unread.
    let r = send(
        h.addr(),
        request(
            "POST",
            "/auth/exchange",
            &[
                ("Host", &host),
                ("Origin", &origin),
                ("Content-Type", "application/json"),
                ("Content-Length", "4096"),
            ],
            b"",
        ),
    )
    .await;
    assert_eq!(r.status, 413, "{r:?}");
    // Chunked, so not declared: refused while it is read.
    let mut body = Vec::new();
    for _ in 0..4 {
        body.extend_from_slice(b"200\r\n");
        body.extend_from_slice(&[b'a'; 0x200]);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(b"0\r\n\r\n");
    let r = send(
        h.addr(),
        request(
            "POST",
            "/auth/exchange",
            &[
                ("Host", &host),
                ("Origin", &origin),
                ("Content-Type", "application/json"),
                ("Transfer-Encoding", "chunked"),
            ],
            &body,
        ),
    )
    .await;
    assert_eq!(r.status, 413, "{r:?}");
}

pub async fn what_is_not_ours_is_not_found_whatever_is_asked(s: &dyn Subject) {
    let h = Harness::new(s).await;
    let host = h.host();
    for (method, path) in [
        ("GET", "/nope"),
        ("POST", "/nope"),
        ("DELETE", "/etc/passwd"),
    ] {
        let r = send(h.addr(), request(method, path, &[("Host", &host)], b"")).await;
        assert_eq!(r.status, 404, "{method} {path}: {r:?}");
    }
    // Even here the guard speaks first about a foreign origin.
    let r = send(
        h.addr(),
        request(
            "POST",
            "/nope",
            &[("Host", &host), ("Origin", "http://evil.example")],
            b"",
        ),
    )
    .await;
    assert_eq!(r.status, 403, "{r:?}");
}

/// Makes a `#[tokio::test]` of each test of the matrix, for the subject the expression gives. The
/// crate that calls it needs `tokio` with `macros` and `rt-multi-thread`.
///
/// ```ignore
/// mod my_app {
///     appkit_testkit::security_matrix!(my_app_testkit::MyApp);
/// }
/// ```
#[macro_export]
macro_rules! security_matrix {
    ($subject:expr) => {
        $crate::security_matrix!(@each $subject;
        a_wrong_host_is_forbidden,
        the_loopback_names_are_accepted,
        two_host_headers_are_a_bad_request,
        a_mutation_needs_a_good_origin,
        cross_site_requests_are_forbidden,
        a_same_origin_get_without_origin_header_gets_through,
        a_browser_mutation_without_origin_is_refused_but_a_program_may,
        there_is_no_cors,
        every_data_route_needs_a_session,
        the_control_secret_and_a_session_are_not_interchangeable,
        a_code_is_not_a_token,
        a_mutation_must_be_json,
        a_body_over_the_limit_is_refused_unread,
        a_chunked_body_over_the_limit_is_refused_while_it_is_read,
        an_overlong_request_line_or_header_is_refused_and_the_server_lives,
        hello_needs_no_token_and_carries_no_data,
        the_page_needs_no_token_and_has_its_headers,
        every_answer_carries_the_security_headers,
        a_pairing_code_is_good_once,
        a_pairing_code_expires,
        a_pairing_code_cannot_be_got_without_the_control_secret,
        a_token_is_never_echoed,
        exchange_attempts_are_rate_limited,
        mutations_are_rate_limited,
        the_server_binds_loopback_only,
        the_connection_limit_closes_the_extra_connection,
        a_client_that_sends_nothing_is_dropped,
        a_silent_connection_is_dropped,
        an_idle_keep_alive_connection_is_dropped,
        the_routes_that_take_a_few_bytes_take_only_a_few_bytes,
        what_is_not_ours_is_not_found_whatever_is_asked,
        );
    };
    (@each $subject:expr; $($name:ident),* $(,)?) => {
        $(
            #[tokio::test]
            async fn $name() {
                $crate::security::$name(&$subject).await
            }
        )*
    };
}

// ── device keys (`GuardRules::with_devices`) ────────────────────────────────

/// A device as a page makes it: a P-256 key pair; the public half as SEC1 uncompressed hex.
struct TestDevice {
    key: p256::ecdsa::SigningKey,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl TestDevice {
    fn new(seed: u8) -> Self {
        Self {
            key: p256::ecdsa::SigningKey::from_slice(&[seed; 32]).expect("a key"),
        }
    }

    fn public(&self) -> String {
        hex(self.key.verifying_key().to_sec1_point(false).as_bytes())
    }

    fn sign(&self, origin: &str, device: &str, challenge: &str) -> String {
        use p256::ecdsa::signature::Signer;
        let message = appkit_hub::devices::login_message(origin, device, challenge);
        let sig: p256::ecdsa::Signature = self.key.sign(message.as_bytes());
        hex(&sig.to_bytes())
    }
}

async fn device_pair(h: &Harness, code: &str, name: &str, key: &str) -> Reply {
    let origin = h.origin();
    let body = serde_json::json!({ "code": code, "name": name, "key": key }).to_string();
    post(
        h,
        appkit_hub::guard::DEVICE_PAIR_PATH,
        &[("Origin", &origin)],
        &body,
    )
    .await
}

/// A challenge and a signed session for `device`, signed for `sign_origin`, sent with the hub's origin.
async fn device_login(h: &Harness, device: &TestDevice, id: &str, sign_origin: &str) -> Reply {
    let origin = h.origin();
    let body = serde_json::json!({ "device": id }).to_string();
    let r = post(
        h,
        appkit_hub::guard::DEVICE_CHALLENGE_PATH,
        &[("Origin", &origin)],
        &body,
    )
    .await;
    assert_eq!(r.status, 200, "{r:?}");
    let challenge = r.json()["challenge"]
        .as_str()
        .expect("a challenge")
        .to_owned();
    let body = serde_json::json!({
        "device": id,
        "challenge": challenge,
        "signature": device.sign(sign_origin, id, &challenge),
    })
    .to_string();
    post(
        h,
        appkit_hub::guard::DEVICE_SESSION_PATH,
        &[("Origin", &origin)],
        &body,
    )
    .await
}

/// The status of a data route with this session: 401 when the hub does not take it; anything else when it does (an app
/// without an engine answers 503, which still means the session got past the guard).
async fn data_status(h: &Harness, s: &dyn Subject, token: &str) -> u16 {
    let auth = bearer(token);
    get(h, &h.route(s.data_routes()[0]), &[("Authorization", &auth)])
        .await
        .status
}

/// An app that keeps devices: a device pairs with a one-time code and its public key and gets a session; later it opens
/// sessions with a signed challenge, as often as it starts, without a code; the pairing outlives a restart of the hub
/// (the list is a file); a signature for another origin or another key opens nothing.
pub async fn a_device_pairs_once_and_then_logs_in_with_its_key(s: &dyn Subject) {
    let device = TestDevice::new(7);
    let id = {
        let h = Harness::new(s).await;
        let code = mint_code(&h).await;
        let r = device_pair(&h, &code, "Phone", &device.public()).await;
        assert_eq!(r.status, 200, "{r:?}");
        let id = r.json()["device"].as_str().expect("a device id").to_owned();
        let token = r.json()["token"].as_str().expect("a token").to_owned();
        assert_ne!(
            data_status(&h, s, &token).await,
            401,
            "the session is taken"
        );
        // The code is spent, also for a second pairing.
        let again = device_pair(&h, &code, "Phone 2", &TestDevice::new(8).public()).await;
        assert_eq!(again.status, 401, "{again:?}");
        id
    };
    // Another run of the hub: its sessions are gone, the device is not.
    let h = Harness::new(s).await;
    let r = device_login(&h, &device, &id, &h.origin()).await;
    assert_eq!(r.status, 200, "{r:?}");
    let token = r.json()["token"].as_str().expect("a token").to_owned();
    assert_ne!(
        data_status(&h, s, &token).await,
        401,
        "the session is taken"
    );
    // Signed for another origin: a page elsewhere relaying the challenge gets nothing.
    let r = device_login(&h, &device, &id, "http://127.0.0.1:1").await;
    assert_eq!(r.status, 401, "{r:?}");
    // Another key under this id.
    let r = device_login(&h, &TestDevice::new(9), &id, &h.origin()).await;
    assert_eq!(r.status, 401, "{r:?}");
    assert!(!r.body.contains("challenge"), "{}", r.body);
}

/// A bad name or key is refused before the code is spent; a pairing route without an Origin (a program) or with a
/// foreign one is refused like `/auth/exchange`.
pub async fn device_pairing_checks_before_it_spends_the_code(s: &dyn Subject) {
    let h = Harness::new(s).await;
    let code = mint_code(&h).await;
    let good = TestDevice::new(7).public();
    let bidi = format!("a{}b", char::from_u32(0x202E).expect("a char"));
    for (name, key) in [
        ("", good.as_str()),
        ("Phone", "04"),
        (bidi.as_str(), good.as_str()),
    ] {
        let r = device_pair(&h, &code, name, key).await;
        assert_eq!(r.status, 400, "{name:?} {key}: {r:?}");
    }
    let body = serde_json::json!({ "code": code, "name": "Phone", "key": good }).to_string();
    let r = post(&h, appkit_hub::guard::DEVICE_PAIR_PATH, &[], &body).await;
    assert_eq!(r.status, 403, "no Origin: {r:?}");
    let r = post(
        &h,
        appkit_hub::guard::DEVICE_PAIR_PATH,
        &[("Origin", "http://evil.example")],
        &body,
    )
    .await;
    assert_eq!(r.status, 403, "foreign Origin: {r:?}");
    // The code still works.
    let r = device_pair(&h, &code, "Phone", &good).await;
    assert_eq!(r.status, 200, "{r:?}");
}

/// The PC lists and revokes devices with the control secret, never with a session; a revoked device's sessions end
/// at once and it cannot log in again.
pub async fn the_pc_revokes_a_device_and_its_sessions_end(s: &dyn Subject) {
    let h = Harness::new(s).await;
    let device = TestDevice::new(7);
    let code = mint_code(&h).await;
    let r = device_pair(&h, &code, "Phone", &device.public()).await;
    let id = r.json()["device"].as_str().expect("a device id").to_owned();
    let token = r.json()["token"].as_str().expect("a token").to_owned();
    let control = bearer(&h.control);
    let session = bearer(&token);
    let revoke_body = serde_json::json!({ "device": id }).to_string();
    // A session (the phone's own) cannot list or revoke.
    for path in [
        appkit_hub::guard::CONTROL_DEVICES_PATH,
        appkit_hub::guard::CONTROL_REVOKE_PATH,
    ] {
        let r = post(&h, path, &[("Authorization", &session)], &revoke_body).await;
        assert_eq!(r.status, 401, "{path}: {r:?}");
    }
    let r = post(
        &h,
        appkit_hub::guard::CONTROL_DEVICES_PATH,
        &[("Authorization", &control)],
        "{}",
    )
    .await;
    assert_eq!(r.status, 200, "{r:?}");
    let list = r.json();
    assert_eq!(list["devices"][0]["id"], id.as_str());
    assert_eq!(list["devices"][0]["name"], "Phone");
    assert!(
        !r.body.contains(&device.public()),
        "the list shows no key: {}",
        r.body
    );
    let r = post(
        &h,
        appkit_hub::guard::CONTROL_REVOKE_PATH,
        &[("Authorization", &control)],
        &revoke_body,
    )
    .await;
    assert_eq!(r.status, 200, "{r:?}");
    assert_eq!(r.json()["revoked"], true);
    assert_eq!(
        data_status(&h, s, &token).await,
        401,
        "the session of the revoked device ends"
    );
    let r = device_login(&h, &device, &id, &h.origin()).await;
    assert_eq!(r.status, 401, "{r:?}");
}

/// Another app (a share target, a link) opens the installable page: the browser sends `Sec-Fetch-Site: cross-site`
/// with a navigation to a document. That opens the page below `prefix` and nothing else: not the desktop page, not a
/// fetch, not a script, not a route of data.
pub async fn another_app_may_open_the_installable_page_and_nothing_else(
    s: &dyn Subject,
    prefix: &str,
) {
    let h = Harness::new(s).await;
    let nav = [
        ("Sec-Fetch-Site", "cross-site"),
        ("Sec-Fetch-Mode", "navigate"),
        ("Sec-Fetch-Dest", "document"),
    ];
    let page = format!("{prefix}/");
    let r = get(&h, &format!("{page}?share_title=x"), &nav).await;
    assert_eq!(r.status, 200, "{r:?}");
    let r = get(&h, "/", &nav).await;
    assert_eq!(r.status, 403, "the desktop page: {r:?}");
    let fetch = [
        ("Sec-Fetch-Site", "cross-site"),
        ("Sec-Fetch-Mode", "cors"),
        ("Sec-Fetch-Dest", "empty"),
    ];
    let r = get(&h, &page, &fetch).await;
    assert_eq!(r.status, 403, "a fetch from elsewhere: {r:?}");
    let script = [
        ("Sec-Fetch-Site", "cross-site"),
        ("Sec-Fetch-Mode", "no-cors"),
        ("Sec-Fetch-Dest", "script"),
    ];
    let r = get(&h, &format!("{prefix}/sw.js"), &script).await;
    assert_eq!(r.status, 403, "a script from elsewhere: {r:?}");
    let token = session(&h).await;
    let auth = bearer(&token);
    let mut with_session = nav.to_vec();
    with_session.push(("Authorization", &auth));
    let r = get(&h, &h.route(s.data_routes()[0]), &with_session).await;
    assert_eq!(
        r.status, 403,
        "a route of data, even as a navigation: {r:?}"
    );
}

/// An app that keeps no devices has none of their routes.
pub async fn without_devices_there_are_no_device_routes(s: &dyn Subject) {
    let h = Harness::new(s).await;
    let origin = h.origin();
    for path in [
        appkit_hub::guard::DEVICE_PAIR_PATH,
        appkit_hub::guard::DEVICE_CHALLENGE_PATH,
        appkit_hub::guard::DEVICE_SESSION_PATH,
    ] {
        let r = post(&h, path, &[("Origin", &origin)], "{}").await;
        assert_eq!(r.status, 404, "{path}: {r:?}");
    }
    let control = bearer(&h.control);
    let r = post(
        &h,
        appkit_hub::guard::CONTROL_DEVICES_PATH,
        &[("Authorization", &control)],
        "{}",
    )
    .await;
    assert_eq!(r.status, 404, "{r:?}");
}
