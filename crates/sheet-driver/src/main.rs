//! Drives the window of a program for a test sheet of testing.nvim, the way a screen reader does.
//!
//! `testing-sheet-driver/1` on stdin and stdout (the schema lies in testing.nvim): one JSON line per
//! command, one per answer, a command is finished before the next one is read, the end of stdin
//! means close. testing.nvim starts the program and names its process here; this driver only
//! touches that process, and it ends nothing: it never starts or stops a program.
//!
//!   cargo build -p sheet-driver --release
//!   TESTING_DESKTOP_DRIVER=<the exe> testing sheets step <sheet>

mod protocol;
#[cfg(windows)]
mod uia;

use std::io::{BufRead, Write};

fn main() {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    #[cfg(windows)]
    let mut desktop = uia::Uia::default();
    #[cfg(not(windows))]
    let mut desktop = protocol::NoDesktop;
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let (answer, more) = protocol::handle(&mut desktop, &line);
        let written = writeln!(stdout, "{answer}").and_then(|()| stdout.flush());
        if written.is_err() || !more {
            break;
        }
    }
}
