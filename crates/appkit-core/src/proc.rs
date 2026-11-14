//! Run a child process with an empty environment, bounded output and a watchdog.
//!
//! The older face of [`crate::child`], kept for the apps that call it: [`run`] is
//! [`crate::child::run`] with an empty environment, a program path that is taken as it is and
//! [`crate::child::Leftovers::Keep`]. New code uses `child`, which also names the caller.

use std::ffi::OsString;
use std::fmt;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

use crate::child::{self, Environment, Leftovers, Start};

/// What to run.
#[derive(Debug, Clone)]
pub struct Spec {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    /// The whole environment of the child. Nothing is inherited.
    pub env: Vec<(OsString, OsString)>,
    pub cwd: Option<PathBuf>,
    /// Bytes written to stdin, then stdin is closed.
    pub stdin: Vec<u8>,
    /// The watchdog: the child is killed when it runs longer.
    pub timeout: Duration,
    pub max_stdout: usize,
    pub max_stderr: usize,
}

/// What the child left behind.
#[derive(Debug, Clone)]
pub struct Output {
    /// `None` when the child was ended by a signal.
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Why a run did not produce an [`Output`].
#[derive(Debug)]
pub enum ProcError {
    /// The program could not be started.
    Spawn(io::Error),
    /// Reading or writing a pipe failed.
    Io(io::Error),
    /// The watchdog fired; the child was killed.
    Timeout(Duration),
    /// A stream grew past its limit; the child was killed.
    TooLarge { stream: &'static str, limit: usize },
}

impl fmt::Display for ProcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(e) => write!(f, "could not start the process: {e}"),
            Self::Io(e) => write!(f, "process pipe failed: {e}"),
            Self::Timeout(d) => write!(f, "the process ran longer than {} ms", d.as_millis()),
            Self::TooLarge { stream, limit } => {
                write!(f, "the process wrote more than {limit} bytes to {stream}")
            }
        }
    }
}

impl std::error::Error for ProcError {}

/// Run `spec` to the end, or kill it.
///
/// # Errors
///
/// As [`crate::child::run`].
pub async fn run(spec: Spec) -> Result<Output, ProcError> {
    let mut start = Start::as_given("appkit_core::proc::run", spec.program)
        .args(spec.args)
        .env(Environment::Only(spec.env));
    if let Some(cwd) = spec.cwd {
        start = start.cwd(cwd);
    }
    child::run(child::Run {
        start,
        stdin: spec.stdin,
        timeout: spec.timeout,
        max_stdout: spec.max_stdout,
        max_stderr: spec.max_stderr,
        // A normal end keeps what the child started on purpose, as this function always did.
        leftovers: Leftovers::Keep,
    })
    .await
}
