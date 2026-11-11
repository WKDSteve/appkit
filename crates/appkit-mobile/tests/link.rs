//! The link of a phone against a real hub of the kit (`DemoDevices`), over the network the app uses (`NetTransport`),
//! the way an SSH tunnel ends on the phone: `http://127.0.0.1:<port of the hub>`.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use appkit_core::authn::{ManualClock, Secret, CODE_BYTES, TOKEN_BYTES};
use appkit_hub::{HubStatus, Limits as HubLimits, RunningServer};
use appkit_mobile::link::{
    pair, AppRoutes, BoxFuture, Limits, Link, LinkError, Method, NetError, SignError, Signer,
    SoftwareSigner, Transport,
};
use appkit_mobile::net::NetTransport;
use appkit_mobile::way::{HubAddress, PairLink};
use appkit_testkit::subject::{StartConfig, Subject};
use appkit_testkit::subjects::DemoDevices;

struct Hub {
    server: RunningServer,
    control: String,
    _subject: DemoDevices,
}

async fn hub() -> Hub {
    let subject = DemoDevices::new();
    let control = Secret::generate(TOKEN_BYTES).unwrap();
    let server = subject
        .start(StartConfig {
            addr: "127.0.0.1:0".parse().unwrap(),
            limits: HubLimits::default(),
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
        .unwrap();
    Hub {
        server,
        control: control.to_hex(),
        _subject: subject,
    }
}

impl Hub {
    fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.server.addr().port())
    }

    /// What `<program> open` does on the PC: a one-time code with the control secret.
    async fn code(&self) -> String {
        let r = reqwest::Client::new()
            .post(format!("{}/api/control/pair", self.base()))
            .bearer_auth(&self.control)
            .header("Content-Type", "application/json")
            .body("{}")
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        let doc: serde_json::Value = r.json().await.unwrap();
        doc["code"].as_str().unwrap().to_owned()
    }

    async fn revoke(&self, device: &str) {
        let r = reqwest::Client::new()
            .post(format!("{}/api/control/devices/revoke", self.base()))
            .bearer_auth(&self.control)
            .header("Content-Type", "application/json")
            .body(serde_json::json!({ "device": device }).to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
    }

    async fn link(&self) -> PairLink {
        PairLink {
            address: HubAddress::tunnel(self.server.addr().port()).unwrap(),
            app: "demo".to_owned(),
            code: self.code().await,
        }
    }
}

fn routes() -> AppRoutes {
    AppRoutes::new("demo-desktop", "demo")
}

fn net() -> Arc<dyn Transport> {
    Arc::new(NetTransport::new().unwrap())
}

/// A key that counts what it signs.
struct Counting {
    key: SoftwareSigner,
    signed: AtomicUsize,
}

impl Signer for Counting {
    fn public_key(&self) -> BoxFuture<'_, Result<String, SignError>> {
        self.key.public_key()
    }
    fn sign<'a>(&'a self, message: &'a [u8]) -> BoxFuture<'a, Result<String, SignError>> {
        self.signed.fetch_add(1, Ordering::SeqCst);
        self.key.sign(message)
    }
}

#[tokio::test]
async fn a_phone_pairs_reads_logs_in_again_and_is_refused_once_revoked() {
    let hub = hub().await;
    let signer = Arc::new(SoftwareSigner::random());
    let (paired, link) = pair(
        net(),
        signer.clone(),
        None,
        &routes(),
        &hub.link().await,
        "Fairphone",
        Limits::default(),
    )
    .await
    .unwrap();
    assert_eq!(paired.app, "demo");
    let r = link
        .fetch(Method::Get, "/api/demo/items", &[], None)
        .await
        .unwrap();
    assert_eq!((r.status, r.body.as_slice()), (200, b"[]".as_slice()));
    let r = link
        .fetch(Method::Post, "/api/demo/ops", b"{}", None)
        .await
        .unwrap();
    assert_eq!(r.status, 200);
    // The app was ended: a new link from the kept pairing logs in by itself.
    let again = Link::new(net(), signer, routes(), paired.clone(), Limits::default());
    let r = again
        .fetch(Method::Get, "/api/demo/item", &[], None)
        .await
        .unwrap();
    assert_eq!(r.status, 200);
    // A route that is not the app's is not sent.
    assert_eq!(
        again
            .fetch(Method::Post, "/api/control/pair", b"{}", None)
            .await,
        Err(LinkError::NotAllowed)
    );
    // Revoked on the PC: the open session ends, and the next login is refused.
    hub.revoke(&paired.device).await;
    assert_eq!(
        again.fetch(Method::Get, "/api/demo/items", &[], None).await,
        Err(LinkError::Unpaired)
    );
}

#[tokio::test]
async fn nothing_is_signed_for_a_hub_that_names_another_id() {
    let hub = hub().await;
    let counting = Arc::new(Counting {
        key: SoftwareSigner::random(),
        signed: AtomicUsize::new(0),
    });
    let (mut paired, _) = pair(
        net(),
        counting.clone(),
        None,
        &routes(),
        &hub.link().await,
        "Fairphone",
        Limits::default(),
    )
    .await
    .unwrap();
    // The phone paired with another hub that now answers on the same port.
    paired.hub = "ee".repeat(16);
    let link = Link::new(net(), counting.clone(), routes(), paired, Limits::default());
    assert_eq!(
        link.fetch(Method::Get, "/api/demo/items", &[], None).await,
        Err(LinkError::OtherHub)
    );
    assert_eq!(counting.signed.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn pairing_checks_the_app_and_a_code_is_good_once() {
    let hub = hub().await;
    let mut link = hub.link().await;
    let other = AppRoutes::new("tasks-desktop", "demo");
    assert_eq!(
        pair(
            net(),
            Arc::new(SoftwareSigner::random()),
            None,
            &other,
            &link,
            "x",
            Limits::default()
        )
        .await
        .err(),
        Some(LinkError::OtherApp)
    );
    link.app = "tasks".to_owned();
    assert_eq!(
        pair(
            net(),
            Arc::new(SoftwareSigner::random()),
            None,
            &routes(),
            &link,
            "x",
            Limits::default()
        )
        .await
        .err(),
        Some(LinkError::OtherApp)
    );
    link.app = "demo".to_owned();
    pair(
        net(),
        Arc::new(SoftwareSigner::random()),
        None,
        &routes(),
        &link,
        "x",
        Limits::default(),
    )
    .await
    .unwrap();
    let spent = pair(
        net(),
        Arc::new(SoftwareSigner::random()),
        None,
        &routes(),
        &link,
        "y",
        Limits::default(),
    )
    .await
    .err();
    assert!(
        matches!(spent, Some(LinkError::Refused { status: 401, ref code }) if code == "bad_code"),
        "{spent:?}"
    );
}

#[tokio::test]
async fn an_answer_over_the_limit_or_late_is_not_taken() {
    let hub = hub().await;
    let small = Limits {
        timeout: Duration::from_secs(5),
        max_bytes: 8,
    };
    // The hello of the hub is larger than 8 bytes.
    let r = pair(
        net(),
        Arc::new(SoftwareSigner::random()),
        None,
        &routes(),
        &hub.link().await,
        "x",
        small,
    )
    .await;
    assert_eq!(r.err(), Some(LinkError::Net(NetError::TooLarge)));
    // A program that takes the connection and never answers.
    let silent = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = silent.local_addr().unwrap().port();
    let keep = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((s, _)) = silent.accept().await {
            held.push(s);
        }
    });
    let late = Limits {
        timeout: Duration::from_millis(300),
        max_bytes: 1024,
    };
    let link = PairLink {
        address: HubAddress::tunnel(port).unwrap(),
        app: "demo".to_owned(),
        code: "00".repeat(16),
    };
    let r = pair(
        net(),
        Arc::new(SoftwareSigner::random()),
        None,
        &routes(),
        &link,
        "x",
        late,
    )
    .await;
    assert_eq!(r.err(), Some(LinkError::Net(NetError::AnswerLost)));
    keep.abort();
    // Nobody there at all: unreachable, safe to try again.
    let closed = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let link = PairLink {
        address: HubAddress::tunnel(closed).unwrap(),
        ..link
    };
    let r = pair(
        net(),
        Arc::new(SoftwareSigner::random()),
        None,
        &routes(),
        &link,
        "x",
        Limits::default(),
    )
    .await;
    assert!(
        matches!(r.err(), Some(LinkError::Net(NetError::Unreachable(_)))),
        "nobody listens"
    );
}

#[tokio::test]
async fn after_a_session_the_kept_copy_holds_no_private_data_and_serves_when_the_hub_is_gone() {
    use appkit_mobile::cache::{Cache, CacheLimits};
    let hub = hub().await;
    let dir = appkit_scratch::Scratch::new("mobile-cache-session");
    let cache = Cache::open(dir.join("cache.json"), CacheLimits::default(), 100);
    let signer = Arc::new(SoftwareSigner::random());
    let (paired, link) = pair(
        net(),
        signer.clone(),
        None,
        &routes(),
        &hub.link().await,
        "Fairphone",
        Limits::default(),
    )
    .await
    .unwrap();
    let r = link
        .read("/api/demo/classified", &cache, 100)
        .await
        .unwrap();
    assert!(!r.kept);
    assert!(r.body.contains("open-area") && r.body.contains("own-area"));
    // A route the app keeps for the PC is refused and not kept.
    let r = link.read("/api/demo/pc-only", &cache, 101).await.unwrap();
    assert_eq!(r.status, 403);
    let kept = std::fs::read_to_string(dir.join("cache.json")).unwrap();
    for (name, class) in appkit_testkit::subjects::CLASSIFIED {
        let released = matches!(class, Some("public" | "internal"));
        assert_eq!(kept.contains(name), released, "{name}: {kept}");
    }
    assert!(!kept.contains("pc-only"), "{kept}");
    // The hub is gone: the kept copy is given, marked as kept, with its time.
    hub.server.shutdown().await;
    let gone = Link::new(net(), signer, routes(), paired, Limits::default());
    let r = gone
        .read("/api/demo/classified", &cache, 200)
        .await
        .unwrap();
    assert!(r.kept);
    assert_eq!(r.fetched_at, 100);
    assert!(r.body.contains("own-area"));
}

#[test]
fn the_signed_texts_are_the_hubs_byte_for_byte() {
    assert_eq!(
        appkit_mobile::link::login_message("o", "d", "c"),
        appkit_hub::devices::login_message("o", "d", "c")
    );
    assert_eq!(
        appkit_mobile::link::confirm_message("o", "d", "c", "POST", "/api/x?y=1", "ab"),
        appkit_hub::devices::confirm_message("o", "d", "c", "POST", "/api/x?y=1", "ab")
    );
}

/// What the person does at the prompt: confirm, or cancel.
struct Prompt {
    key: SoftwareSigner,
    yes: bool,
    asked: AtomicUsize,
}

impl Signer for Prompt {
    fn public_key(&self) -> BoxFuture<'_, Result<String, SignError>> {
        self.key.public_key()
    }
    fn sign<'a>(&'a self, message: &'a [u8]) -> BoxFuture<'a, Result<String, SignError>> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        if self.yes {
            self.key.sign(message)
        } else {
            Box::pin(async { Err(SignError::Cancelled) })
        }
    }
}

#[tokio::test]
async fn a_trigger_runs_only_at_its_level_and_with_the_persons_confirmation() {
    let hub = hub().await;
    let prompt = Arc::new(Prompt {
        key: SoftwareSigner::random(),
        yes: true,
        asked: AtomicUsize::new(0),
    });
    let login: Arc<SoftwareSigner> = Arc::new(SoftwareSigner::random());
    let (paired, link) = pair(
        net(),
        login.clone(),
        Some(prompt.clone()),
        &routes(),
        &hub.link().await,
        "Fairphone",
        Limits::default(),
    )
    .await
    .unwrap();
    // A new device may not trigger: the hub says so, after the person confirmed on the phone.
    let r = link
        .trigger("/api/demo/run", br#"{"key":"a"}"#)
        .await
        .unwrap();
    assert_eq!(r.status, 403);
    assert!(String::from_utf8_lossy(&r.body).contains("level_too_low"));
    // The PC raises the level.
    let r = reqwest::Client::new()
        .post(format!("{}/api/control/devices/level", hub.base()))
        .bearer_auth(&hub.control)
        .header("Content-Type", "application/json")
        .body(serde_json::json!({ "device": paired.device, "level": "trigger" }).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let r = link
        .trigger("/api/demo/run", br#"{"key":"a"}"#)
        .await
        .unwrap();
    assert_eq!(
        (r.status, r.body.as_slice()),
        (200, br#"{"key":"a"}"#.as_slice())
    );
    assert_eq!(prompt.asked.load(Ordering::SeqCst), 2);
    // A plain request to a trigger route carries no confirmation: refused.
    let r = link
        .fetch(Method::Post, "/api/demo/run", b"{}", None)
        .await
        .unwrap();
    assert_eq!(r.status, 403);
    // The person cancels at the prompt: nothing is sent, the hub has nothing to refuse.
    let cancel = Arc::new(Prompt {
        key: SoftwareSigner::random(),
        yes: false,
        asked: AtomicUsize::new(0),
    });
    let no = Link::new(
        net(),
        login.clone(),
        routes(),
        paired.clone(),
        Limits::default(),
    )
    .with_confirm(Some(cancel.clone()));
    assert_eq!(
        no.trigger("/api/demo/run", b"{}").await.err(),
        Some(LinkError::Sign(SignError::Cancelled))
    );
    assert_eq!(cancel.asked.load(Ordering::SeqCst), 1);
    let without = Link::new(net(), login, routes(), paired, Limits::default());
    assert_eq!(
        without.trigger("/api/demo/run", b"{}").await.err(),
        Some(LinkError::Sign(SignError::Gone)),
        "a phone without a confirmation key never triggers"
    );
}

#[test]
fn the_link_the_pc_shows_is_the_one_the_phone_reads() {
    use appkit_hub::phone::{pair_link, PhoneWay};
    let code = "0123456789abcdef0123456789abcdef";
    let tailnet = pair_link(
        &PhoneWay::Tailnet("pc.example.ts.net".to_owned()),
        17618,
        "testing",
        code,
    );
    let read = appkit_mobile::way::parse_pair_link(&tailnet).expect("read back");
    assert_eq!(
        read.address,
        HubAddress::tailnet("pc.example.ts.net").unwrap()
    );
    assert_eq!((read.app.as_str(), read.code.as_str()), ("testing", code));
    let tunnel = pair_link(&PhoneWay::Tunnel, 17618, "testing", code);
    assert_eq!(
        appkit_mobile::way::parse_pair_link(&tunnel)
            .unwrap()
            .address,
        HubAddress::tunnel(17618).unwrap()
    );
}
