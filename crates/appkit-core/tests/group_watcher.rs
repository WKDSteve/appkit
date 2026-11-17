//! The watcher of a child's process group (`src/child/unix.rs`): when the app is killed (SIGKILL, so none of its own
//! code runs), the group of its child ends, the child and what the child started; a group that was released
//! (`Leftovers::Keep`) lives on; and the watcher holds none of the app's other pipes.
//!
//! The app is this test program itself, started again with `APPKIT_WATCHER_APP` naming a folder: then the test
//! [`app`] starts the child through `child`, writes the process ids it wants checked into the folder and waits to be
//! killed. Without that variable [`app`] does nothing.
#![cfg(unix)]
#![allow(unsafe_code)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use appkit_core::child::{self, Environment, Leftovers, Run, Start};

const APP: &str = "APPKIT_WATCHER_APP";
const WHAT: &str = "APPKIT_WATCHER_WHAT";

fn sh() -> Start {
    Start::new("appkit test group watcher", "/bin/sh").env(Environment::Only(vec![(
        "PATH".into(),
        "/usr/bin:/bin".into(),
    )]))
}

/// The app's side, in the program started by [`kill_the_app`].
#[tokio::test]
async fn app() {
    let Some(dir) = std::env::var_os(APP).map(PathBuf::from) else {
        return;
    };
    let what = std::env::var(WHAT).unwrap_or_default();
    // `$$` is the shell, the leader of the group; `$!` the grandchild it leaves running.
    let script = format!(
        "sleep 600 & echo $$ $! > '{}/pids.tmp' && mv '{}/pids.tmp' '{}/pids'; wait",
        dir.display(),
        dir.display(),
        dir.display()
    );
    let _held = match what.as_str() {
        "spawn" => Some(child::spawn(&sh().args(["-c", &script])).expect("the child starts")),
        "keep" => {
            // A call that leaves a process running on purpose: once it ended, the group is the person's.
            let script = format!(
                "sleep 600 > /dev/null 2>&1 & echo $$ $! > '{}/pids.tmp' && mv '{}/pids.tmp' '{}/pids'",
                dir.display(),
                dir.display(),
                dir.display()
            );
            child::run(Run {
                start: sh().args(["-c", &script]),
                stdin: Vec::new(),
                timeout: Duration::from_secs(30),
                max_stdout: 1024,
                max_stderr: 1024,
                leftovers: Leftovers::Keep,
            })
            .await
            .expect("the call ends");
            None
        }
        other => panic!("{WHAT}={other}"),
    };
    std::fs::write(dir.join("ready"), "").unwrap();
    tokio::time::sleep(Duration::from_secs(600)).await;
}

/// Whether `pid` still runs (a zombie, ended but not yet reaped by whoever adopted it, does not).
fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 only asks whether the process exists.
    if unsafe { libc::kill(pid, 0) } != 0 {
        return false;
    }
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        // The state is the field after the name, which is in parentheses and may hold spaces.
        Ok(stat) => stat
            .rsplit_once(')')
            .and_then(|(_, rest)| rest.split_whitespace().next())
            .is_some_and(|state| state != "Z"),
        // No /proc (macOS): the process exists.
        Err(_) => true,
    }
}

fn wait_for(path: &Path, what: &str) {
    let end = Instant::now() + Duration::from_secs(30);
    while !path.exists() {
        assert!(Instant::now() < end, "{what} did not appear");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Start this program as the app with `what`, wait until its child is under way, kill the app and give back the
/// two process ids its child wrote: the child's own and the grandchild's.
fn kill_the_app(dir: &Path, what: &str) -> (i32, i32) {
    let mut app = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "app", "--nocapture", "--test-threads=1"])
        .env(APP, dir)
        .env(WHAT, what)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    wait_for(&dir.join("ready"), "the app's child");
    let pids = std::fs::read_to_string(dir.join("pids")).unwrap();
    let pids: Vec<i32> = pids
        .split_whitespace()
        .map(|p| p.parse().unwrap())
        .collect();
    assert_eq!(pids.len(), 2, "{pids:?}");
    // SIGKILL: nothing of the app runs any more, no drop, no cleanup.
    app.kill().unwrap();
    app.wait().unwrap();
    (pids[0], pids[1])
}

#[test]
fn a_killed_app_takes_the_group_of_its_child_with_it() {
    let scratch = appkit_scratch::Scratch::new("group-watcher");
    let (child, grandchild) = kill_the_app(scratch.path(), "spawn");
    let end = Instant::now() + Duration::from_secs(10);
    while alive(child) || alive(grandchild) {
        assert!(
            Instant::now() < end,
            "still running after the app was killed: child {child} {}, grandchild {grandchild} {}",
            alive(child),
            alive(grandchild)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_released_group_outlives_the_killed_app() {
    let scratch = appkit_scratch::Scratch::new("group-watcher");
    let (_, left) = kill_the_app(scratch.path(), "keep");
    std::thread::sleep(Duration::from_secs(1));
    let lives = alive(left);
    // SAFETY: the process the call left running on purpose, ended by the test that made it.
    unsafe { libc::kill(left, libc::SIGKILL) };
    assert!(lives, "the process the call kept was ended with the app");
}

/// The watcher of a second child is forked while the app holds the stdin of the first: it closes it, so the first
/// child still sees the end of its input when the app closes its end.
#[tokio::test]
async fn the_watcher_of_one_child_holds_no_pipe_of_another() {
    let mut first = child::spawn(&sh().args(["-c", "cat > /dev/null"])).unwrap();
    let stdin = first.take_stdin().unwrap();
    let second = child::spawn(&sh().args(["-c", "sleep 600"])).unwrap();
    // The watcher of the second child is forked by now (the start waits for the middle process).
    drop(stdin);
    let status = tokio::time::timeout(Duration::from_secs(10), first.wait()).await;
    drop(second);
    let status = status.expect("the first child did not see the end of its input");
    assert!(status.unwrap().success());
}
