//! A program to be started by `appkit_core::child` in a test (`tests/child.rs`). The first
//! argument says what it plays:
//!
//! - `args <a>...`: prints each argument on a line of its own.
//! - `env <NAME>`: prints the value of the variable, or `(unset)`.
//! - `cat`: copies stdin to stdout.
//! - `flood`: writes to stdout without end.
//! - `lines`: writes `out 1` to stdout and `err 1` to stderr, waits, then `out 2` with a carriage
//!   return and `tail` without a line end.
//! - `hub <helper|plain> <out>`: plays a hub that has no console: asks a `console` child of its
//!   own, through the helper or by hand, and writes the answer to `<out>`.
//! - `sleep`: does nothing for a minute.
//! - `console`: prints `window` when its console has a window, else `none` (Windows; elsewhere
//!   `none`).
//! - `beat <file>`: appends a byte to `<file>` every 50 ms, for half a minute at most or until
//!   `<file>.stop` exists. A test sees a process live by the file growing.
//! - `tree <file>`: starts `beat <file>` as a process of its own and then sleeps.
//! - `leave <file>`: starts `beat <file>` as a process of its own and ends.
//! - `launch <file>`: as `leave`, but the way a hub starts a program of somebody else
//!   (`appkit_hub::launch`): it holds no pipe of this program.
//! - `start`: plays `<hub> start` for `background::start_from` (`tests/start_from.rs`): starts
//!   `beat $CHILD_PROBE_BEAT` the way `launch` does, prints what a hub prints and whether its
//!   console has a window, warns on stderr; fails with `CHILD_PROBE_FAIL` set.
//! - `app <role>...`: plays an app: starts itself with `<role>...` through `child::spawn`,
//!   prints `started` and sleeps. The test ends it the hard way.
//! - `bare <program> <arg>...`: starts `<program>` without standard handles, the way a double
//!   click, a link or a shell in a terminal starts a window program, waits for it and ends with
//!   its exit code. The console of this probe (or none) is what the program finds at its parent.
#![allow(unsafe_code)]

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use appkit_core::child::{self, Environment, Start};
use appkit_hub::launch::{self, LaunchOpts, Window};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (role, rest) = args.split_first().expect("the first argument is the role");
    match role.as_str() {
        "args" => {
            for arg in rest {
                println!("{arg}");
            }
        }
        "env" => {
            let value = std::env::var(&rest[0]).unwrap_or_else(|_| "(unset)".to_owned());
            println!("{value}");
        }
        "cat" => {
            let mut text = Vec::new();
            std::io::stdin().read_to_end(&mut text).expect("read stdin");
            std::io::stdout().write_all(&text).expect("write stdout");
        }
        "flood" => {
            let block = [b'x'; 4096];
            let mut out = std::io::stdout();
            while out.write_all(&block).is_ok() {}
        }
        "lines" => {
            println!("out 1");
            eprintln!("err 1");
            std::thread::sleep(Duration::from_millis(300));
            print!("out 2\r\ntail");
            std::io::stdout().flush().expect("write stdout");
        }
        "hub" => hub(&rest[0], &rest[1]),
        "sleep" => std::thread::sleep(Duration::from_secs(60)),
        "console" => println!("{}", if console_window() { "window" } else { "none" }),
        "beat" => beat(&rest[0]),
        "tree" => {
            start_beat(&rest[0]);
            std::thread::sleep(Duration::from_secs(60));
        }
        "leave" => start_beat(&rest[0]),
        "launch" => {
            let me = std::env::current_exe().expect("the path of this program");
            let cwd = std::env::current_dir().expect("a working folder");
            let opts = LaunchOpts {
                cwd: &cwd,
                window: Window::Default,
            };
            launch::spawn(&me, &["beat", &rest[0]], &opts).expect("start the beat");
        }
        "app" => app(rest),
        "start" => start(),
        "bare" => bare(rest),
        other => panic!("unknown role {other}"),
    }
}

/// Starts `role[0]` with `role[1..]` and null standard handles: this probe gives up its own
/// first, and a stream inherited as null stays null in the child.
fn bare(role: &[String]) {
    #[cfg(windows)]
    for which in [
        windows_sys::Win32::System::Console::STD_INPUT_HANDLE,
        windows_sys::Win32::System::Console::STD_OUTPUT_HANDLE,
        windows_sys::Win32::System::Console::STD_ERROR_HANDLE,
    ] {
        // SAFETY: no pointer; a null standard handle is what a process started without any has.
        let _ = unsafe {
            windows_sys::Win32::System::Console::SetStdHandle(which, std::ptr::null_mut())
        };
    }
    let status = Command::new(&role[0])
        .args(&role[1..])
        .status()
        .expect("start the program");
    std::process::exit(status.code().unwrap_or(1));
}

/// Plays `<hub> start`: puts a "hub" (a `beat`) in the background the way `launch` does, says so
/// on stdout the way a hub does, and warns on stderr.
fn start() {
    if let Ok(file) = std::env::var("CHILD_PROBE_BEAT") {
        let me = std::env::current_exe().expect("the path of this program");
        let cwd = std::env::current_dir().expect("a working folder");
        let opts = LaunchOpts {
            cwd: &cwd,
            window: Window::Default,
        };
        launch::spawn(&me, &["beat", &file], &opts).expect("start the beat");
    }
    eprintln!("a warning of the probe");
    if std::env::var_os("CHILD_PROBE_FAIL").is_some() {
        std::process::exit(3);
    }
    let console = if console_window() { "window" } else { "none" };
    println!("the hub runs in the background (probe, console {console})");
}

fn beat(file: &str) {
    let end = Instant::now() + Duration::from_secs(30);
    let stop = format!("{file}.stop");
    while Instant::now() < end && !std::path::Path::new(&stop).exists() {
        let written = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(file)
            .and_then(|mut f| f.write_all(b"."));
        // The folder of the test is gone: so is the reason to live.
        if written.is_err() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A process of its own, in the group and the job of this one. Nobody waits for it: this
/// program ends or sleeps.
#[allow(clippy::zombie_processes)]
fn start_beat(file: &str) {
    Command::new(std::env::current_exe().expect("the path of this program"))
        .args(["beat", file])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start the beat");
}

fn hub(how: &str, out: &str) {
    let me = std::env::current_exe().expect("the path of this program");
    let answer = if how == "helper" {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        let run = child::Run {
            start: Start::new("child-probe hub", me)
                .arg("console")
                .env(Environment::Inherited {
                    set: Vec::new(),
                    remove: Vec::new(),
                }),
            stdin: Vec::new(),
            timeout: Duration::from_secs(30),
            max_stdout: 1024,
            max_stderr: 1024,
            leftovers: child::Leftovers::End,
        };
        runtime.block_on(child::run(run)).expect("the child").stdout
    } else {
        Command::new(me)
            .arg("console")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .expect("the child")
            .stdout
    };
    std::fs::write(out, answer).expect("write the answer");
}

fn app(role: &[String]) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a runtime");
    runtime.block_on(async {
        let me = std::env::current_exe().expect("the path of this program");
        let start = Start::new("child-probe app", me)
            .args(role)
            .env(Environment::Inherited {
                set: Vec::new(),
                remove: Vec::new(),
            });
        let _child = child::spawn(&start).expect("start the child");
        println!("started");
        tokio::time::sleep(Duration::from_secs(60)).await;
    });
}

/// Whether the console of this process has a window.
#[cfg(windows)]
fn console_window() -> bool {
    // SAFETY: no arguments; the answer is only compared with null.
    !unsafe { windows_sys::Win32::System::Console::GetConsoleWindow() }.is_null()
}

#[cfg(not(windows))]
fn console_window() -> bool {
    false
}
