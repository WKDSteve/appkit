//! Run a child process with an empty environment, bounded output and a watchdog.
//!
//! Nothing here goes through a shell: the program and its arguments are
//! passed as they are, and the environment is exactly the list the caller
//! gives (`env_clear` first). A child that outlives its time or writes more
//! than the limit is killed, never waited for.

use std::ffi::OsString;
use std::fmt;
use std::io;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

/// How long a killed child may take to be reaped.
const REAP_GRACE: Duration = Duration::from_secs(2);

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

/// Read at most `limit` bytes; one byte more than that is an error.
async fn read_limited<R>(
    mut reader: R,
    limit: usize,
    stream: &'static str,
) -> Result<Vec<u8>, ProcError>
where
    R: AsyncRead + Unpin,
{
    let mut buf = Vec::new();
    let mut chunk = [0u8; 16 * 1024];
    loop {
        let n = reader.read(&mut chunk).await.map_err(ProcError::Io)?;
        if n == 0 {
            return Ok(buf);
        }
        if buf.len() + n > limit {
            return Err(ProcError::TooLarge { stream, limit });
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

async fn write_stdin<W>(mut writer: W, bytes: Vec<u8>) -> Result<(), ProcError>
where
    W: AsyncWriteExt + Unpin,
{
    match writer.write_all(&bytes).await {
        Ok(()) => {}
        // The child ended before it read everything: its exit status says why.
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => return Ok(()),
        Err(e) => return Err(ProcError::Io(e)),
    }
    match writer.shutdown().await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(e) => Err(ProcError::Io(e)),
    }
}

/// Run `spec` to the end, or kill it.
pub async fn run(spec: Spec) -> Result<Output, ProcError> {
    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .env_clear()
        .envs(spec.env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(cwd) = &spec.cwd {
        command.current_dir(cwd);
    }
    #[cfg(windows)]
    {
        // CREATE_NO_WINDOW: no console window flashes up for a console program.
        command.creation_flags(0x0800_0000);
    }

    let mut child = command.spawn().map_err(ProcError::Spawn)?;
    // Dropped on every way out but a normal end: the processes the child started end with it.
    let tree = crate::job::Job::adopt(&child);
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (Some(stdin), Some(stdout), Some(stderr)) = (stdin, stdout, stderr) else {
        let _ = child.start_kill();
        return Err(ProcError::Io(io::Error::other(
            "a pipe of the child is missing",
        )));
    };

    let pipes = async {
        tokio::try_join!(
            write_stdin(stdin, spec.stdin),
            read_limited(stdout, spec.max_stdout, "stdout"),
            read_limited(stderr, spec.max_stderr, "stderr"),
        )
    };

    // One deadline for the whole run: the pipes and the wait share it, so the
    // watchdog never grants the time twice.
    let deadline = tokio::time::Instant::now() + spec.timeout;
    let outcome = match tokio::time::timeout_at(deadline, pipes).await {
        Ok(Ok(((), out, err))) => {
            // The pipes are closed, so the child is ending. Bound the wait too.
            match tokio::time::timeout_at(deadline, child.wait()).await {
                Ok(Ok(status)) => {
                    // It ended as it should: what it started on purpose lives on.
                    if let Some(tree) = tree {
                        tree.release();
                    }
                    return Ok(Output {
                        code: status.code(),
                        stdout: out,
                        stderr: err,
                    });
                }
                Ok(Err(e)) => Err(ProcError::Io(e)),
                Err(_) => Err(ProcError::Timeout(spec.timeout)),
            }
        }
        Ok(Err(e)) => Err(e),
        Err(_) => Err(ProcError::Timeout(spec.timeout)),
    };

    // Every path that gets here ends the child: it is never left running.
    let _ = child.start_kill();
    let _ = tokio::time::timeout(REAP_GRACE, child.wait()).await;
    outcome
}
