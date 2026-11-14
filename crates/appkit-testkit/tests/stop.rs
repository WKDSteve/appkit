//! `appkit_hub::background::stop_in` against a hub that is a process of its own (`hub-probe`):
//! the hub is asked to end itself first, and ended by the system when it does not. No program is
//! started for either (`taskkill.exe`, `/bin/kill`).

use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use appkit_core::registry::{self, HubRecord};
use appkit_hub::background::stop_in;
use appkit_scratch::Scratch;
use appkit_testkit::subjects::DEMO_IDENTITY as IDENTITY;

const PROBE: &str = env!("CARGO_BIN_EXE_hub-probe");

/// Start the probe hub and wait until it wrote its record. The probe is a child of this test, as
/// a hub is a child of the program that put it in the background and stops it again: when it has
/// ended it keeps its id until the test waits for it, and `stop` must still see that it is gone.
// Every test hands the child to `ended`, which waits for it; a test that fails before leaves a
// probe that ends by itself.
#[allow(clippy::zombie_processes)]
fn hub(dir: &Path, mode: &str) -> (Child, HubRecord) {
    let child = Command::new(PROBE)
        .arg(dir)
        .arg(mode)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("the probe starts");
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(Some(record)) = registry::read_record(dir) {
            assert_eq!(record.pid, child.id());
            return (child, record);
        }
        assert!(Instant::now() < end, "the probe hub wrote no record");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The exit status of the probe, which must have ended by now.
fn ended(mut child: Child) -> ExitStatus {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().expect("look at the probe") {
            return status;
        }
        if Instant::now() >= end {
            let _ = child.kill();
            let _ = child.wait();
            panic!("the probe hub still runs after `stop`");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[tokio::test]
async fn a_hub_that_listens_ends_itself_when_it_is_asked() {
    let dir = Scratch::new("stop-listen");
    let (child, record) = hub(&dir, "listen");

    let said = stop_in(&IDENTITY, &dir, Duration::from_secs(20))
        .await
        .unwrap();
    assert_eq!(
        said,
        format!("the hub (process {}) is stopped\n", record.pid)
    );
    // It ended by itself, in an orderly way: it wrote its mark and its exit code is its own. A
    // process that was ended by the system does neither.
    assert!(dir.join("ended").is_file(), "the hub was ended, not asked");
    assert_eq!(ended(child).code(), Some(0));
    assert!(!dir.join("hub.json").exists());
    assert!(!dir.join("control.secret").exists());

    let again = stop_in(&IDENTITY, &dir, Duration::from_secs(1)).await;
    assert_eq!(again.unwrap(), "no hub is running\n");
}

#[tokio::test]
async fn a_hub_that_does_not_listen_is_ended_by_the_system() {
    let dir = Scratch::new("stop-deaf");
    let (child, record) = hub(&dir, "deaf");

    let asked = Instant::now();
    let said = stop_in(&IDENTITY, &dir, Duration::from_millis(500))
        .await
        .unwrap();
    assert_eq!(
        said,
        format!("the hub (process {}) is stopped\n", record.pid)
    );
    assert!(
        asked.elapsed() >= Duration::from_millis(500),
        "the hub was not given its time to end itself"
    );
    assert!(!dir.join("ended").exists());
    // Ended from outside: not the exit code of a program that returned from `main`.
    assert_ne!(ended(child).code(), Some(0));
    assert!(!dir.join("hub.json").exists());
    assert!(!dir.join("control.secret").exists());
}

/// A hub without its secret file cannot be asked: it is ended by the system at once, as a hub
/// of an older version that does not know the route.
#[tokio::test]
async fn a_hub_that_cannot_be_asked_is_ended_without_waiting() {
    let dir = Scratch::new("stop-no-secret");
    let (child, record) = hub(&dir, "listen");
    registry::remove_secret(&dir);

    let asked = Instant::now();
    let said = stop_in(&IDENTITY, &dir, Duration::from_secs(60))
        .await
        .unwrap();
    assert!(said.contains(&record.pid.to_string()), "{said}");
    assert!(
        asked.elapsed() < Duration::from_secs(30),
        "waited for nothing"
    );
    assert!(!dir.join("ended").exists());
    assert_ne!(ended(child).code(), Some(0));
}

/// A record that names a living process which is not the hub (here: this test) ends nobody.
#[tokio::test]
async fn a_stale_record_ends_nobody() {
    let dir = Scratch::new("stop-stale");
    let (child, record) = hub(&dir, "deaf");
    // The record now names this process, on the port of the hub: the hub answers with its
    // instance, but the record is of somebody else.
    let wrong_instance = HubRecord {
        instance: "0".repeat(record.instance.len()),
        pid: std::process::id(),
        ..record.clone()
    };
    registry::write_record(&dir, &wrong_instance).unwrap();
    let said = stop_in(&IDENTITY, &dir, Duration::from_millis(100)).await;
    assert_eq!(said.unwrap(), "no hub is running\n");

    // The right instance with the id of this process: this process is never ended.
    let own_pid = HubRecord {
        pid: std::process::id(),
        ..record.clone()
    };
    registry::write_record(&dir, &own_pid).unwrap();
    let err = stop_in(&IDENTITY, &dir, Duration::from_millis(100))
        .await
        .unwrap_err();
    assert!(err.contains("not the id of another process"), "{err}");

    // The hub still runs; now it is stopped for real.
    registry::write_record(&dir, &record).unwrap();
    stop_in(&IDENTITY, &dir, Duration::from_millis(100))
        .await
        .unwrap();
    ended(child);
}
