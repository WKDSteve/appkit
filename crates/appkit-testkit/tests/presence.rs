//! Who is here, through the real server: a connection counts from the request on which the guard
//! accepted a session until it ends, and nothing else counts. A hub in the background ends itself
//! by this count, so a request that anybody can send must never keep one alive.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use appkit_core::authn::{ManualClock, Secret, CODE_BYTES, TOKEN_BYTES};
use appkit_hub::{HubStatus, Limits, RunningServer};
use appkit_testkit::subject::{StartConfig, Subject};
use appkit_testkit::subjects::Demo;
use appkit_testkit::wire::{parse, request, send, Reply};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

async fn start() -> (RunningServer, String) {
    let control = Secret::generate(TOKEN_BYTES).unwrap();
    let server = Demo
        .start(StartConfig {
            addr: "127.0.0.1:0".parse().unwrap(),
            limits: Limits::default(),
            clock: Arc::new(ManualClock::new()),
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
    (server, control.to_hex())
}

fn host(addr: SocketAddr) -> String {
    format!("127.0.0.1:{}", addr.port())
}

/// A session, got the way `<program> open` and the page get one.
async fn session(addr: SocketAddr, control: &str) -> String {
    let host = host(addr);
    let auth = format!("Bearer {control}");
    let code = send(
        addr,
        request(
            "POST",
            "/api/control/pair",
            &[
                ("Host", &host),
                ("Authorization", &auth),
                ("Content-Type", "application/json"),
                ("Content-Length", "2"),
            ],
            b"{}",
        ),
    )
    .await;
    assert_eq!(code.status, 200, "{code:?}");
    let code = code.json()["code"].as_str().expect("a code").to_owned();
    let body = format!(r#"{{"code":"{code}"}}"#);
    let origin = format!("http://{host}");
    let length = body.len().to_string();
    let token = send(
        addr,
        request(
            "POST",
            "/auth/exchange",
            &[
                ("Host", &host),
                ("Origin", &origin),
                ("Content-Type", "application/json"),
                ("Content-Length", &length),
            ],
            body.as_bytes(),
        ),
    )
    .await;
    assert_eq!(token.status, 200, "{token:?}");
    token.json()["token"].as_str().expect("a token").to_owned()
}

/// One request on a connection that stays open afterwards, as the connection of a page does.
async fn keep_alive_get(stream: &mut TcpStream, addr: SocketAddr, path: &str, auth: &str) -> Reply {
    let text = format!(
        "GET {path} HTTP/1.1\r\nHost: {}\r\nAuthorization: {auth}\r\n\r\n",
        host(addr)
    );
    stream.write_all(text.as_bytes()).await.expect("write");
    let mut buf = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    // The head and a body of the length it names.
    loop {
        let mut chunk = [0u8; 4096];
        let n = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut chunk))
            .await
            .expect("an answer in time")
            .expect("read");
        buf.extend_from_slice(&chunk[..n]);
        let reply = parse(&buf);
        let whole = reply
            .header("content-length")
            .and_then(|l| l.parse::<usize>().ok())
            .is_some_and(|l| reply.body.len() >= l);
        if whole || n == 0 {
            return reply;
        }
        assert!(Instant::now() < deadline, "no whole answer: {reply:?}");
    }
}

async fn alone_within(server: &RunningServer, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if server.presence().alone_for().is_some() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

#[tokio::test]
async fn somebody_is_here_while_a_connection_with_a_session_is_open() {
    let (server, control) = start().await;
    let addr = server.addr();
    let token = session(addr, &control).await;
    // Pairing itself is nobody: the control secret and the exchange carry no session. But a pairing code is a
    // page on its way, so the time alone began again with it.
    assert!(alone_within(&server, Duration::from_secs(5)).await);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let before = server.presence().alone_for().expect("nobody is here");
    assert!(before >= Duration::from_millis(300), "{before:?}");
    let _again = session(addr, &control).await;
    let after = server.presence().alone_for().expect("nobody is here");
    assert!(after < before, "{after:?} is not less than {before:?}");

    let mut page = TcpStream::connect(addr).await.expect("connect");
    let reply = keep_alive_get(&mut page, addr, "/api/vaults", &format!("Bearer {token}")).await;
    assert_ne!(reply.status, 401, "the session was accepted: {reply:?}");
    assert!(
        server.presence().alone_for().is_none(),
        "a page with a session is connected"
    );

    // It stays here as long as the connection is open, whatever else comes and goes.
    let hello = send(
        addr,
        request("GET", "/api/demo/hello", &[("Host", &host(addr))], b""),
    )
    .await;
    assert_eq!(hello.status, 200);
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(server.presence().alone_for().is_none());

    drop(page);
    assert!(
        alone_within(&server, Duration::from_secs(5)).await,
        "the connection ended: nobody is here"
    );
    server.shutdown().await;
}

#[tokio::test]
async fn what_anybody_can_send_is_nobody() {
    let (server, _control) = start().await;
    let addr = server.addr();
    let began = Instant::now();

    // `hello`, the page, and a request the guard refuses, each on a connection that stays open.
    let mut asked = Vec::new();
    for (path, auth) in [
        ("/api/demo/hello", "Bearer none"),
        ("/", "Bearer none"),
        (
            "/api/vaults",
            "Bearer 0000000000000000000000000000000000000000000000000000000000000000",
        ),
    ] {
        let mut stream = TcpStream::connect(addr).await.expect("connect");
        let reply = keep_alive_get(&mut stream, addr, path, auth).await;
        if path == "/api/vaults" {
            assert_eq!(reply.status, 401, "{reply:?}");
        } else {
            assert_eq!(reply.status, 200, "{path}: {reply:?}");
        }
        asked.push(stream);
    }
    let alone = server.presence().alone_for().expect("nobody is here");
    // Alone since the start: none of them moved the time either.
    assert!(
        alone + Duration::from_millis(50) >= began.elapsed(),
        "{alone:?}"
    );
    drop(asked);
    server.shutdown().await;
}

/// `<program> stop` asks the hub to end over its control route. Only the control secret does
/// that: a session of a page does not, and nobody without a credential.
#[tokio::test]
async fn the_hub_is_asked_to_end_with_the_control_secret_only() {
    let (server, control) = start().await;
    let addr = server.addr();
    let token = session(addr, &control).await;
    let host = host(addr);
    let origin = format!("http://{host}");
    let stop = appkit_hub::guard::CONTROL_STOP_PATH;
    let asked = |server: &RunningServer| {
        let presence = server.presence();
        async move {
            tokio::time::timeout(Duration::from_millis(200), presence.asked_to_end())
                .await
                .is_ok()
        }
    };
    let post = |auth: Option<String>, with_origin: bool| {
        let mut headers = vec![
            ("Host", host.clone()),
            ("Content-Type", "application/json".to_owned()),
            ("Content-Length", "2".to_owned()),
        ];
        if let Some(auth) = auth {
            headers.push(("Authorization", auth));
        }
        if with_origin {
            headers.push(("Origin", origin.clone()));
        }
        async move {
            let headers: Vec<(&str, &str)> =
                headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
            send(addr, request("POST", stop, &headers, b"{}")).await
        }
    };

    for (auth, with_origin) in [
        (None, false),
        (Some(format!("Bearer {token}")), true),
        (Some(format!("Bearer {token}")), false),
        (Some(format!("Bearer {}", "0".repeat(control.len()))), false),
    ] {
        let reply = post(auth.clone(), with_origin).await;
        assert_eq!(reply.status, 401, "{auth:?}: {reply:?}");
    }
    assert!(!asked(&server).await, "somebody without the secret asked");

    let reply = post(Some(format!("Bearer {control}")), false).await;
    assert_eq!(reply.status, 200, "{reply:?}");
    assert_eq!(reply.json()["ending"], true);
    assert!(asked(&server).await);
    // The app ends by the same wait it ends by when it is left alone, whatever the order to stay.
    tokio::time::timeout(
        Duration::from_secs(5),
        server
            .presence()
            .left_alone(Duration::from_secs(3600), || true),
    )
    .await
    .expect("left_alone resolves for a hub that was asked to end");
    server.shutdown().await;
}

#[tokio::test]
async fn a_hub_without_devices_has_none_paired() {
    let (server, _control) = start().await;
    assert_eq!(server.paired_devices(), 0);
    server.shutdown().await;
}
