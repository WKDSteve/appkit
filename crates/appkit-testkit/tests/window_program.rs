//! `appkit_core::console` from outside, with a real window program (`window-probe`): what it reads
//! from its header, where its output goes, which console it attaches to and that its exit code
//! reaches whoever waits for it.
//!
//! A parent with a console and one without are played by `child-probe bare`, which starts the
//! window program without standard handles, as a shell in a terminal or a double click does.
#![cfg(windows)]

use std::os::windows::process::CommandExt as _;
use std::process::{Command, Stdio};

use appkit_scratch::Scratch;
use appkit_testkit::window_program::{subsystem, CONSOLE, WINDOW};

const WINDOW_PROBE: &str = env!("CARGO_BIN_EXE_window-probe");
const CHILD_PROBE: &str = env!("CARGO_BIN_EXE_child-probe");

/// `CREATE_NO_WINDOW`: a console program gets a console, without a window.
const NO_WINDOW: u32 = 0x0800_0000;
/// `DETACHED_PROCESS`: a console program gets no console at all.
const DETACHED: u32 = 0x0000_0008;

fn read(path: &str) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

#[test]
fn the_header_tells_a_window_program_from_a_console_program() {
    assert_eq!(subsystem(&read(WINDOW_PROBE)), Some(WINDOW));
    assert_eq!(subsystem(&read(CHILD_PROBE)), Some(CONSOLE));
    // Programs of Windows that are known to be one and the other.
    let root = std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap());
    let cmd = std::fs::read(root.join("System32").join("cmd.exe")).unwrap();
    let explorer = std::fs::read(root.join("explorer.exe")).unwrap();
    assert_eq!(subsystem(&cmd), Some(CONSOLE));
    assert_eq!(subsystem(&explorer), Some(WINDOW));
}

/// All three streams go somewhere: nothing is attached, the output stays in the pipes, and the
/// exit code is the program's (a script that captures the output waits for it).
#[test]
fn output_to_pipes_stays_there_and_the_exit_code_is_the_programs() {
    let out = Command::new(WINDOW_PROBE)
        .args(["echo", "7"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout), "out\nnone\n");
    assert_eq!(String::from_utf8_lossy(&out.stderr), "err\n");
    assert_eq!(out.status.code(), Some(7));
}

#[test]
fn output_redirected_to_a_file_goes_to_the_file() {
    let dir = Scratch::new("window-program-file");
    let file = dir.join("out.txt");
    let out = Command::new(WINDOW_PROBE)
        .args(["echo", "0"])
        .stdin(Stdio::null())
        .stdout(std::fs::File::create(&file).unwrap())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "out\nnone\n");
    assert_eq!(String::from_utf8_lossy(&out.stderr), "err\n");
}

/// Starts the window program through `child-probe bare` with `flags` and returns what it wrote.
fn state_under(flags: u32, name: &str) -> String {
    let dir = Scratch::new(name);
    let file = dir.join("state.txt");
    let status = Command::new(CHILD_PROBE)
        .arg("bare")
        .arg(WINDOW_PROBE)
        .arg("state")
        .arg(&file)
        .creation_flags(flags)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert_eq!(
        status.code(),
        Some(3),
        "the exit code of the window program reaches the parent that waits"
    );
    std::fs::read_to_string(&file).unwrap()
}

/// Started from a terminal (a parent with a console, no streams handed over): the program writes
/// to that console, and stderr is read there.
#[test]
fn started_from_a_console_it_attaches_to_it() {
    assert_eq!(
        state_under(NO_WINDOW, "window-program-parent"),
        "parent true true"
    );
}

/// Started by a double click or a link (a parent without a console): no console, and stderr goes
/// nowhere (an error would be shown in a message box). Writing to stdout does not fail.
#[test]
fn started_without_a_console_it_has_none_and_stderr_is_not_read() {
    assert_eq!(
        state_under(DETACHED, "window-program-none"),
        "none true false"
    );
}

/// The window for the questions of a first start: opened, used, closed, and afterwards the program
/// writes where it wrote before. Opens a visible console window for a moment, so it runs by hand:
/// `cargo test -p appkit-testkit --test window_program -- --ignored`.
#[test]
#[ignore = "opens a visible console window"]
fn a_window_of_its_own_opens_and_closes_again() {
    let dir = Scratch::new("window-program-own");
    let file = dir.join("own.txt");
    let status = Command::new(WINDOW_PROBE)
        .arg("own")
        .arg(&file)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "true own none");

    // From a terminal: the window, then the console of the terminal again.
    let file = dir.join("own-from-terminal.txt");
    let status = Command::new(CHILD_PROBE)
        .arg("bare")
        .arg(WINDOW_PROBE)
        .arg("own")
        .arg(&file)
        .creation_flags(NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "true own parent");
}
