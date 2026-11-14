//! `appkit_hub::background::start_from` from outside: `<program> start` is a child of the caller
//! while it runs (no console window, an absolute path, a line in the log), and the hub it puts in
//! the background stays when the call is over.
//!
//! The program is `child-probe start` (`src/bin/child-probe.rs`). One test only: it sets
//! variables of this process, which the probe inherits.
#![allow(unsafe_code)]

use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use appkit_core::child;
use appkit_hub::background::{start_from, started_now};
use appkit_scratch::Scratch;

const PROBE: &str = env!("CARGO_BIN_EXE_child-probe");

static LOGGED: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn size(file: &Path) -> u64 {
    std::fs::metadata(file).map(|m| m.len()).unwrap_or(0)
}

/// The file grows: the process that writes it lives.
fn beats(file: &Path) -> bool {
    let before = size(file);
    let end = Instant::now() + Duration::from_secs(20);
    while Instant::now() < end {
        if size(file) > before {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[test]
fn the_installed_program_is_a_child_while_it_starts_its_hub_and_the_hub_stays() {
    assert!(child::set_log(|line| {
        eprintln!("{line}");
        LOGGED.lock().unwrap().push(line.to_owned());
    }));
    let dir = Scratch::new("start-from");
    let beat = dir.join("beat");
    // SAFETY: the only test of this program, so no other thread reads the environment.
    unsafe { std::env::set_var("CHILD_PROBE_BEAT", &beat) };

    // Outside of a runtime.
    let printed = start_from(Path::new(PROBE)).unwrap();
    assert!(started_now(&printed), "{printed}");
    // The child has a console without a window (Windows); a start by hand would have handed
    // it the console of the test run, or a new one.
    #[cfg(windows)]
    assert!(printed.contains("console none"), "{printed}");
    assert_eq!(printed.lines().count(), 1, "{printed}");

    // Caller and program are in the log, as for every child of the app.
    let logged = LOGGED.lock().unwrap().clone();
    let mine: Vec<&String> = logged
        .iter()
        .filter(|l| l.contains("start of the installed hub"))
        .collect();
    assert_eq!(mine.len(), 1, "{logged:?}");
    assert!(mine[0].starts_with("child started by "), "{logged:?}");
    assert!(mine[0].contains("child-probe"), "{logged:?}");

    // What the program put in the background is not ended with the call.
    assert!(beats(&beat), "the hub of the probe did not stay");
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        beats(&beat),
        "the hub of the probe was ended after the call"
    );

    // Inside a runtime the call blocks and answers the same.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let again = runtime.block_on(async { start_from(Path::new(PROBE)) });
    assert!(started_now(&again.unwrap()));

    // A name is not searched on PATH.
    let err = start_from(Path::new("child-probe")).unwrap_err();
    assert!(err.contains("not an absolute path"), "{err}");

    // A program that fails is an error.
    // SAFETY: as above.
    unsafe { std::env::set_var("CHILD_PROBE_FAIL", "1") };
    let err = start_from(Path::new(PROBE)).unwrap_err();
    assert!(err.contains("did not come back"), "{err}");

    // The beats end by themselves when asked.
    std::fs::write(dir.join("beat.stop"), b"").unwrap();
}
