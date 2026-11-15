//! A window program (`windows_subsystem = "windows"` in every build, unlike a hub, whose debug build
//! keeps its console), for the tests of `appkit_core::console` (`tests/window_program.rs`). The first
//! argument says what it plays:
//!
//! - `echo <code>`: attaches, writes `out` to stdout and `err` to stderr, then the console it got
//!   (`own`, `parent`, `none`) to stdout, and ends with `<code>`.
//! - `state <file>`: attaches and writes `<console> <started without streams> <stderr is read>` to
//!   `<file>`, then a line to stdout (which must not fail, wherever stdout goes); ends with 3.
//! - `own <file>`: opens a console window of its own, notes what it has then, closes it again and
//!   writes both to `<file>`. A visible window for a moment: only by hand (an ignored test).
//! - `--version` and `install <args>`: a hub as `install.ps1` of the release kit calls it
//!   (`tests/install_ps1.rs`). Each waits a moment first, so a caller that does not wait for it is
//!   seen: `--version` prints `window-probe 1.0`; `install` writes its arguments to the file
//!   `WINDOW_PROBE_INSTALLED` names, prints `installed` and ends with `WINDOW_PROBE_EXIT` (0).
#![windows_subsystem = "windows"]

use std::io::Write as _;

use appkit_core::console::{self, Console};

fn name(console: Console) -> &'static str {
    match console {
        Console::Own => "own",
        Console::Parent => "parent",
        Console::None => "none",
    }
}

fn main() {
    let got = console::attach_parent();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (role, rest) = args.split_first().expect("the first argument is the role");
    match role.as_str() {
        "echo" => {
            println!("out");
            eprintln!("err");
            println!("{}", name(got));
            std::process::exit(rest[0].parse().expect("an exit code"));
        }
        "state" => {
            let line = format!(
                "{} {} {}",
                name(got),
                console::started_without_streams(),
                console::stderr_is_read()
            );
            std::fs::write(&rest[0], line).expect("write the answer");
            let _ = writeln!(std::io::stdout(), "written");
            std::process::exit(3);
        }
        "own" => {
            let opened = console::open_own("window-probe");
            let during = format!("{opened} {}", name(console::attach_parent()));
            println!("in a window of its own");
            console::close_own();
            let after = name(console::attach_parent());
            std::fs::write(&rest[0], format!("{during} {after}")).expect("write the answer");
        }
        "--version" => {
            std::thread::sleep(std::time::Duration::from_millis(500));
            println!("window-probe 1.0");
        }
        "install" => {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let file = std::env::var_os("WINDOW_PROBE_INSTALLED").expect("WINDOW_PROBE_INSTALLED");
            std::fs::write(file, format!("install {}", rest.join(" "))).expect("write the answer");
            println!("installed");
            let code = std::env::var("WINDOW_PROBE_EXIT").map_or(Ok(0), |c| c.parse());
            std::process::exit(code.expect("WINDOW_PROBE_EXIT is a number"));
        }
        other => panic!("unknown role {other}"),
    }
}
