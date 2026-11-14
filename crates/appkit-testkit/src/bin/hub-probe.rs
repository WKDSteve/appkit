//! A hub of the demo app as a process of its own, for the tests of `background::stop`
//! (`tests/stop.rs`): it serves on a free port and writes `hub.json` and `control.secret` into
//! `<dir>`, as a hub does.
//!
//! `hub-probe <dir> listen`: ends itself when it is asked to (`/api/control/stop`), the way an app
//! does that waits for `left_alone`; it writes `<dir>/ended` first and exits with 0.
//!
//! `hub-probe <dir> deaf`: answers the request and stays, as a hub that does not wait for it. It
//! ends by itself after a minute, so that a test that fails leaves nothing behind.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use appkit_core::authn::{ManualClock, Secret, CODE_BYTES, TOKEN_BYTES};
use appkit_core::registry::{self, HubRecord};
use appkit_hub::{HubStatus, Limits};
use appkit_testkit::subject::{StartConfig, Subject};
use appkit_testkit::subjects::Demo;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (dir, mode) = match args.as_slice() {
        [dir, mode] => (Path::new(dir), mode.as_str()),
        _ => panic!("hub-probe <dir> <listen|deaf>"),
    };
    let control = Secret::generate(TOKEN_BYTES).expect("a secret");
    let instance = Secret::generate(CODE_BYTES).expect("an instance");
    let server = Demo
        .start(StartConfig {
            addr: "127.0.0.1:0".parse().expect("an address"),
            limits: Limits::default(),
            clock: Arc::new(ManualClock::new()),
            pairing_ttl: Duration::from_secs(60),
            control_secret: control.clone(),
            instance,
            status: HubStatus {
                engine_ready: true,
                problem: None,
                capabilities: Vec::new(),
            },
        })
        .await
        .expect("the server starts");
    registry::write_secret(dir, &control.to_hex()).expect("write the secret");
    let record = HubRecord {
        app: Demo.identity().app.to_owned(),
        port: server.addr().port(),
        pid: std::process::id(),
        version: "0.0.0".to_owned(),
        instance: server.instance().to_owned(),
    };
    registry::write_record(dir, &record).expect("write the record");

    match mode {
        "listen" => {
            server.presence().asked_to_end().await;
            server.shutdown().await;
            std::fs::write(dir.join("ended"), b"").expect("write the mark");
        }
        "deaf" => tokio::time::sleep(Duration::from_secs(60)).await,
        other => panic!("unknown mode {other}"),
    }
}
