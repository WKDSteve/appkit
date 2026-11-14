//! The one way an app starts a process for its own work.
//!
//! A child started here belongs to the app, and that is visible from outside:
//!
//! - It has no console window (`CREATE_NO_WINDOW` on Windows). That alone would only hide it,
//!   so the rest of the list is not optional.
//! - It ends with the app, also when the app is killed or crashes. Windows: the child is put
//!   into a job object with `KILL_ON_JOB_CLOSE`; the system closes the job when the app is
//!   gone and ends every process in it. Linux and macOS: the child leads a process group of
//!   its own, which is killed when the [`Owned`] is dropped, the time is over or the output is
//!   too large; on Linux the child also gets the parent-death signal. See "Limits" below.
//! - Its three standard streams are pipes of the app. It never gets a console, a log handle
//!   or a terminal of its parent.
//! - The program is a path and its arguments are an argv. No shell splits a string, and no
//!   program is started under another name or copied to another place first.
//! - The log (stderr unless [`set_log`] names another place) says who asked (`caller`), which
//!   program and which process id: for every child that stays, and for the first call of each
//!   pair of caller and program (a hub asks its engine thousands of times; the log of a hub is
//!   not cut while it runs). A child that had to be ended (time, size) always gets a line.
//!   Arguments are not logged: they can hold paths and data.
//!
//! Three shapes: [`run`] for a call that ends by itself (its output is collected, with a time
//! limit and a size limit), [`run_lines`] for one whose output is shown line by line while it
//! runs, and [`spawn`] for a child that stays (an engine host), which gives an [`Owned`] with
//! the three pipes.
//!
//! This module is for a child the app owns. A program that is meant to outlive the app (another
//! app, an editor the person works in) is started through `appkit_hub::launch`, visibly and
//! without a job; a link or a folder is handed to the system (`appkit_hub::browser`).
//!
//! Both need a tokio runtime.
//!
//! # Limits
//!
//! - Windows: the child runs for a moment before it is in its job (it is not started
//!   suspended). A process it starts in that moment is not in the job.
//! - Linux: the parent-death signal ends the child itself when the app dies without cleaning
//!   up. What the child started is then ended only if it follows its parent (a pipe that
//!   closes). While the app lives, the whole group is ended.
//! - macOS has no parent-death signal. A child there ends with the app when the app ends in
//!   an orderly way (the [`Owned`] is dropped); after a crash of the app it ends when it next
//!   reads or writes one of its pipes.
//! - A process that leaves its group (`setsid`) or its job on purpose is not ended.
//! - With [`Leftovers::Keep`], a process the child left running that holds one of the child's
//!   pipes keeps the call waiting until the time is over.
//! - Windows gives a console program a console also without a window, so a `conhost.exe`
//!   runs beside such a child and ends with it.

use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};

pub use crate::proc::{Output, ProcError};

#[cfg(unix)]
mod unix;

/// How long a killed child may take to be reaped.
const REAP_GRACE: Duration = Duration::from_secs(2);

/// The environment of a child.
#[derive(Debug, Clone)]
pub enum Environment {
    /// Exactly this list. Nothing is inherited.
    Only(Vec<(OsString, OsString)>),
    /// The environment of the app, without the names in `remove` and with the pairs in `set`.
    Inherited {
        set: Vec<(OsString, OsString)>,
        remove: Vec<OsString>,
    },
}

/// What to start, and who asks.
#[derive(Debug, Clone)]
pub struct Start {
    caller: String,
    program: PathBuf,
    args: Vec<OsString>,
    env: Environment,
    cwd: Option<PathBuf>,
    checked: bool,
}

impl Start {
    /// `caller` names the app and the reason in plain words (`tasks-hub sheet run`); it is
    /// what the log says. `program` is an absolute path to a file on this machine: no name
    /// that is searched on `PATH`, no network share, no `..`.
    ///
    /// The environment is empty until [`Start::env`] says otherwise.
    pub fn new(caller: impl Into<String>, program: impl Into<PathBuf>) -> Self {
        Self {
            caller: caller.into(),
            program: program.into(),
            args: Vec::new(),
            env: Environment::Only(Vec::new()),
            cwd: None,
            checked: true,
        }
    }

    /// As [`Start::new`], for a program a person named in a settings file: the path is taken
    /// as it is and the system decides whether it can be started. New code uses `new`.
    pub fn as_given(caller: impl Into<String>, program: impl Into<PathBuf>) -> Self {
        Self {
            checked: false,
            ..Self::new(caller, program)
        }
    }

    #[must_use]
    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    #[must_use]
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    #[must_use]
    pub fn env(mut self, env: Environment) -> Self {
        self.env = env;
        self
    }

    /// The working folder: absolute and existing (for [`Start::as_given`]: as it is).
    #[must_use]
    pub fn cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    /// Who asks.
    #[must_use]
    pub fn caller(&self) -> &str {
        &self.caller
    }

    /// The program that is started.
    #[must_use]
    pub fn program(&self) -> &Path {
        &self.program
    }

    fn check(&self) -> io::Result<()> {
        let refuse = |why: String| Err(io::Error::new(io::ErrorKind::InvalidInput, why));
        if self.caller.trim().is_empty() {
            return refuse("not started: the caller has no name".to_owned());
        }
        if !self.checked {
            return Ok(());
        }
        if !local_absolute(&self.program) || !self.program.is_file() {
            return refuse(format!(
                "not started: {} is not an absolute path to a file",
                self.program.display()
            ));
        }
        if let Some(cwd) = &self.cwd
            && (!local_absolute(cwd) || !cwd.is_dir())
        {
            return refuse(format!(
                "not started: {} is not an absolute path to a folder",
                cwd.display()
            ));
        }
        Ok(())
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        match &self.env {
            Environment::Only(list) => {
                command.env_clear().envs(list.iter().map(|(k, v)| (k, v)));
            }
            Environment::Inherited { set, remove } => {
                for name in remove {
                    command.env_remove(name);
                }
                command.envs(set.iter().map(|(k, v)| (k, v)));
            }
        }
        if let Some(cwd) = &self.cwd {
            command.current_dir(cwd);
        }
        #[cfg(windows)]
        {
            // CREATE_NO_WINDOW: a console program gets no console and so no window. The child is
            // not hidden by it: it is in a job of this process and named in the log.
            command.creation_flags(0x0800_0000);
        }
        #[cfg(unix)]
        unix::prepare(&mut command);
        command
    }
}

/// Is `path` absolute, on this machine (no network share, no device namespace) and without a
/// `..`?
#[must_use]
pub fn local_absolute(path: &Path) -> bool {
    use std::path::{Component, Prefix};
    path.is_absolute()
        && path.components().all(|c| match c {
            Component::ParentDir => false,
            Component::Prefix(p) => !matches!(
                p.kind(),
                Prefix::UNC(..) | Prefix::VerbatimUNC(..) | Prefix::DeviceNS(_)
            ),
            _ => true,
        })
}

/// What happens to processes the child started and left running when it ended by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leftovers {
    /// They are ended: nothing stays behind a call.
    End,
    /// They live on: the child started them on purpose (an editor the person asked for).
    Keep,
}

/// A call that ends by itself.
#[derive(Debug, Clone)]
pub struct Run {
    pub start: Start,
    /// Bytes written to stdin, then stdin is closed.
    pub stdin: Vec<u8>,
    /// The child and everything it started are ended when the call runs longer.
    pub timeout: Duration,
    /// One byte more on a stream ends the child the same way.
    pub max_stdout: usize,
    pub max_stderr: usize,
    pub leftovers: Leftovers,
}

/// The processes of one child: ended together, at the latest when this is dropped.
#[derive(Debug)]
struct Tree {
    #[cfg(windows)]
    job: Option<crate::job::Job>,
    #[cfg(unix)]
    group: Option<u32>,
}

impl Tree {
    fn adopt(child: &Child) -> Self {
        Self {
            #[cfg(windows)]
            job: crate::job::Job::adopt(child),
            // The child leads its own group (`unix::prepare`), so its id names the group.
            #[cfg(unix)]
            group: child.id(),
        }
    }

    /// End every process of the tree. Once only: afterwards the group id may belong to others.
    fn end(&mut self) {
        #[cfg(windows)]
        drop(self.job.take());
        #[cfg(unix)]
        if let Some(group) = self.group.take() {
            unix::kill_group(group);
        }
    }

    /// Let what is left of the tree live on.
    fn release(&mut self) {
        #[cfg(windows)]
        if let Some(job) = self.job.take() {
            job.release();
        }
        #[cfg(unix)]
        {
            self.group = None;
        }
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        self.end();
    }
}

/// A running child of the app. Dropping it ends the child and everything it started.
#[derive(Debug)]
pub struct Owned {
    child: Child,
    tree: Tree,
    /// Caller, program and process id, for the log.
    what: String,
}

impl Owned {
    /// The process id, while the child has not been waited for.
    #[must_use]
    pub fn id(&self) -> Option<u32> {
        self.child.id()
    }

    /// The pipe to the child's stdin; once.
    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.child.stdin.take()
    }

    /// The pipe from the child's stdout; once. A pipe nobody reads fills up and stops the child.
    pub fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.child.stdout.take()
    }

    /// The pipe from the child's stderr; once. As for stdout: read it.
    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.child.stderr.take()
    }

    /// Wait until the child ends by itself. What it left running is ended then.
    ///
    /// # Errors
    ///
    /// The system could not wait for the child.
    pub async fn wait(&mut self) -> io::Result<ExitStatus> {
        let status = self.child.wait().await?;
        self.tree.end();
        Ok(status)
    }

    /// Whether the child has ended, without waiting. What it left running is ended then.
    ///
    /// # Errors
    ///
    /// The system could not look at the child.
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        let status = self.child.try_wait()?;
        if status.is_some() {
            self.tree.end();
        }
        Ok(status)
    }

    /// End the child and everything it started, now, and collect its exit status.
    pub async fn end(&mut self) {
        // The tree first: on Unix the group is only safe to signal while its leader is not
        // reaped (afterwards the id can be given to another process).
        self.tree.end();
        let _ = self.child.start_kill();
        let _ = tokio::time::timeout(REAP_GRACE, self.child.wait()).await;
    }
}

static LOG: OnceLock<fn(&str)> = OnceLock::new();

/// Name the place the lines of this module go to (the default is stderr, which is the log file
/// of a hub in the background). Once per process; false when it was set before.
pub fn set_log(sink: fn(&str)) -> bool {
    LOG.set(sink).is_ok()
}

fn log(line: &str) {
    match LOG.get() {
        Some(sink) => sink(line),
        None => eprintln!("{line}"),
    }
}

/// A name for a log line: one line, no control characters.
fn plain(text: &OsStr) -> String {
    text.to_string_lossy()
        .chars()
        .map(|c| if c.is_control() { '?' } else { c })
        .collect()
}

/// The pairs of caller and program whose first call is in the log. Bounded: an app has a
/// handful, and past the bound every call is logged again rather than none.
const KNOWN_CALLS: usize = 256;

/// Whether this call is the first of its caller and program (then it is logged).
fn first_call(start: &Start) -> bool {
    static SEEN: OnceLock<Mutex<HashSet<(String, PathBuf)>>> = OnceLock::new();
    let mut seen = SEEN
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if seen.len() >= KNOWN_CALLS {
        return true;
    }
    seen.insert((start.caller.clone(), start.program.clone()))
}

/// Start a child that stays. Must be called in a tokio runtime.
///
/// # Errors
///
/// [`ProcError::Spawn`]: the caller has no name, the program or the working folder is not an
/// absolute local path to a file or folder, there is no runtime, or the system refused to
/// start it.
pub fn spawn(start: &Start) -> Result<Owned, ProcError> {
    let owned = start_child(start)?;
    log(&format!("child started by {}", owned.what));
    Ok(owned)
}

fn start_child(start: &Start) -> Result<Owned, ProcError> {
    start.check().map_err(ProcError::Spawn)?;
    // Without a runtime tokio would panic, not refuse.
    tokio::runtime::Handle::try_current().map_err(|e| ProcError::Spawn(io::Error::other(e)))?;
    let command = start.command();
    #[cfg(unix)]
    let child = unix::spawn(command);
    #[cfg(not(unix))]
    let child = {
        let mut command = command;
        command.spawn()
    };
    let child = child.map_err(ProcError::Spawn)?;
    let tree = Tree::adopt(&child);
    let what = format!(
        "{}: {} (pid {})",
        plain(OsStr::new(&start.caller)),
        plain(start.program.as_os_str()),
        child.id().unwrap_or(0)
    );
    Ok(Owned { child, tree, what })
}

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
    // On the heap: the futures of a call are moved, and two such blocks in each would be copied
    // on the stack every time.
    let mut chunk = vec![0u8; 16 * 1024];
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

/// Which output stream of a child a line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

impl Stream {
    fn name(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        }
    }
}

/// Hand every line of `reader` to `on_line` as it arrives; more than `limit` bytes in all is an
/// error. A line is what stands before a line feed, without a carriage return at its end; bytes
/// that are no UTF-8 become the replacement character.
async fn read_lines<R, F>(
    mut reader: R,
    limit: usize,
    stream: Stream,
    on_line: &Mutex<F>,
) -> Result<(), ProcError>
where
    R: AsyncRead + Unpin,
    F: FnMut(Stream, &str),
{
    let emit = |line: &[u8]| {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let mut on_line = on_line.lock().unwrap_or_else(PoisonError::into_inner);
        on_line(stream, &String::from_utf8_lossy(line));
    };
    let mut pending: Vec<u8> = Vec::new();
    let mut total = 0usize;
    // On the heap: the futures of a call are moved, and two such blocks in each would be copied
    // on the stack every time.
    let mut chunk = vec![0u8; 16 * 1024];
    loop {
        let n = reader.read(&mut chunk).await.map_err(ProcError::Io)?;
        if n == 0 {
            // What the child wrote last without ending the line.
            if !pending.is_empty() {
                emit(&pending);
            }
            return Ok(());
        }
        total += n;
        if total > limit {
            return Err(ProcError::TooLarge {
                stream: stream.name(),
                limit,
            });
        }
        let mut rest = &chunk[..n];
        while let Some(at) = rest.iter().position(|b| *b == b'\n') {
            pending.extend_from_slice(&rest[..at]);
            emit(&pending);
            pending.clear();
            rest = &rest[at + 1..];
        }
        pending.extend_from_slice(rest);
    }
}

/// The part every call shares: wait for the pipes and the end of the child within the time,
/// deal with what the child left running, and end everything when something went wrong.
async fn finish<T>(
    mut owned: Owned,
    timeout: Duration,
    leftovers: Leftovers,
    pipes: impl Future<Output = Result<T, ProcError>>,
) -> Result<(T, ExitStatus), ProcError> {
    // What the child left running is ended as soon as the child itself has ended, not after the
    // pipes: a leftover that inherited a pipe would hold it open until the time is over.
    let child = &mut owned.child;
    let tree = &mut owned.tree;
    let exit = async {
        let status = child.wait().await.map_err(ProcError::Io)?;
        if leftovers == Leftovers::End {
            // On Unix the group is signalled right after its leader was reaped: the id is still
            // taken while a member lives, and free only for the moment between the two calls.
            tree.end();
        }
        Ok(status)
    };

    // One deadline for the whole call: the pipes and the wait share it, so the time is never
    // granted twice.
    let deadline = tokio::time::Instant::now() + timeout;
    let both = Box::pin(async { tokio::try_join!(pipes, exit) });
    let failure = match tokio::time::timeout_at(deadline, both).await {
        Ok(Ok(done)) => {
            if leftovers == Leftovers::Keep {
                owned.tree.release();
            }
            return Ok(done);
        }
        Ok(Err(e)) => e,
        Err(_) => ProcError::Timeout(timeout),
    };

    // Every way that gets here ends the child and what it started.
    log(&format!("child of {} was ended: {failure}", owned.what));
    owned.end().await;
    Err(failure)
}

/// The three pipes of a call that was just started.
async fn call_pipes(
    start: &Start,
) -> Result<(Owned, ChildStdin, ChildStdout, ChildStderr), ProcError> {
    let mut owned = start_child(start)?;
    if first_call(start) {
        log(&format!(
            "child started by {} (later calls of the same kind are not logged)",
            owned.what
        ));
    }
    let pipes = (owned.take_stdin(), owned.take_stdout(), owned.take_stderr());
    let (Some(stdin), Some(stdout), Some(stderr)) = pipes else {
        owned.end().await;
        return Err(ProcError::Io(io::Error::other(
            "a pipe of the child is missing",
        )));
    };
    Ok((owned, stdin, stdout, stderr))
}

/// Run a call to its end and collect what it wrote, or end it. Must be called in a tokio
/// runtime.
///
/// # Errors
///
/// As [`spawn`]; [`ProcError::Timeout`] and [`ProcError::TooLarge`] when a limit was passed
/// (the child and what it started are ended first); [`ProcError::Io`] when a pipe failed.
pub async fn run(run: Run) -> Result<Output, ProcError> {
    let (owned, stdin, stdout, stderr) = call_pipes(&run.start).await?;
    let pipes = async {
        tokio::try_join!(
            write_stdin(stdin, run.stdin),
            read_limited(stdout, run.max_stdout, "stdout"),
            read_limited(stderr, run.max_stderr, "stderr"),
        )
    };
    let (((), out, err), status) = finish(owned, run.timeout, run.leftovers, pipes).await?;
    Ok(Output {
        code: status.code(),
        stdout: out,
        stderr: err,
    })
}

/// As [`run`], for a call whose output is shown while it runs (an update, a clone): every line
/// of stdout and stderr goes to `on_line` as it arrives, and nothing is collected. The size
/// limits count all bytes of a stream. The exit code is the answer; `None` when a signal ended
/// the child.
///
/// # Errors
///
/// As [`run`].
pub async fn run_lines<F>(run: Run, on_line: F) -> Result<Option<i32>, ProcError>
where
    F: FnMut(Stream, &str),
{
    let (owned, stdin, stdout, stderr) = call_pipes(&run.start).await?;
    let on_line = Mutex::new(on_line);
    let pipes = async {
        tokio::try_join!(
            write_stdin(stdin, run.stdin),
            read_lines(stdout, run.max_stdout, Stream::Stdout, &on_line),
            read_lines(stderr, run.max_stderr, Stream::Stderr, &on_line),
        )
    };
    let (_, status) = finish(owned, run.timeout, run.leftovers, pipes).await?;
    Ok(status.code())
}
