//! A watchdog that fires ends the whole process tree of the child, not only the child (Windows
//! job object, `src/job.rs`).
#![cfg(windows)]

use std::ffi::OsString;
use std::time::Duration;

use appkit_core::proc::{run, ProcError, Spec};

#[tokio::test]
async fn timeout_ends_the_grandchild_too() {
    let scratch = appkit_scratch::Scratch::new("proc-tree");
    let dir = scratch.path().to_path_buf();
    let marker = dir.join("marker.txt");
    let script = dir.join("grand.bat");
    // The grandchild writes the marker after three seconds, if it still lives then.
    std::fs::write(
        &script,
        format!(
            "@ping -n 4 127.0.0.1 >nul\r\n@echo x> \"{}\"\r\n",
            marker.display()
        ),
    )
    .unwrap();

    let system = std::env::var_os("SystemRoot").unwrap_or_default();
    let cmd = std::path::Path::new(&system)
        .join("System32")
        .join("cmd.exe");
    let spec = Spec {
        program: cmd,
        // `start /b` leaves the batch file running on its own; the outer cmd then waits.
        args: vec![
            OsString::from("/c"),
            OsString::from(format!(
                "start /b {} & ping -n 30 127.0.0.1 >nul",
                script.display()
            )),
        ],
        env: vec![
            (OsString::from("SystemRoot"), system.clone()),
            (
                OsString::from("PATH"),
                std::path::Path::new(&system)
                    .join("System32")
                    .into_os_string(),
            ),
        ],
        cwd: Some(dir.clone()),
        stdin: Vec::new(),
        timeout: Duration::from_secs(1),
        max_stdout: 1024,
        max_stderr: 1024,
    };
    let outcome = run(spec).await;
    assert!(matches!(outcome, Err(ProcError::Timeout(_))), "{outcome:?}");

    tokio::time::sleep(Duration::from_secs(5)).await;
    let written = marker.exists();
    assert!(!written, "the grandchild outlived the watchdog");
}

/// A long-running child of the system folder, with the job adopted.
fn sleeper() -> tokio::process::Child {
    let system = std::env::var_os("SystemRoot").unwrap_or_default();
    tokio::process::Command::new(
        std::path::Path::new(&system)
            .join("System32")
            .join("ping.exe"),
    )
    .args(["-n", "30", "127.0.0.1"])
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::null())
    .kill_on_drop(true)
    .spawn()
    .unwrap()
}

#[tokio::test]
async fn dropping_the_job_ends_the_process() {
    let mut child = sleeper();
    let job = appkit_core::job::Job::adopt(&child).expect("a job");
    drop(job);
    let status = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
    assert!(status.is_ok(), "the process outlived its job");
}

#[tokio::test]
async fn a_released_job_leaves_the_process_alone() {
    let mut child = sleeper();
    let job = appkit_core::job::Job::adopt(&child).expect("a job");
    job.release();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        child.try_wait().unwrap().is_none(),
        "release ended the process"
    );
    child.kill().await.unwrap();
}
