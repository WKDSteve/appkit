//! Start a program of somebody else: argv only, nothing inherited.
//!
//! A hub that starts other apps (the central hub starts `tasks-hub start --open`, `ai-desktop`,
//! `docmap-desktop`) must not hand them anything of its own. A child that inherits the console
//! or a log handle of its parent runs invisibly, keeps `hub.log` from being truncated, or holds
//! the pipe of whoever started the hub open. So every start goes through [`spawn`]:
//!
//! - the program and its arguments are an argv (`Command::new(program).args(args)`), never a
//!   string a shell would split, and the program is an absolute path to a file;
//! - stdin, stdout and stderr are the null device;
//! - on Windows the standard handles of this process are made not inheritable first (see
//!   `handles`), so no other handle of ours reaches the child either;
//! - the working folder and the kind of window are named by the caller, never defaulted from
//!   whatever this process happens to have.
//!
//! The program path is the caller's responsibility: it must come from a place the hub trusts
//! (its own discovery), never from a request.

use std::path::Path;
use std::process::{Command, Stdio};

/// What window the child gets (Windows; ignored elsewhere, where a program has no console of its
/// own to choose).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Window {
    /// Nothing is asked for: a GUI program makes its own window, a console program shares the
    /// console of this process (`<program> start --open` prints nothing, its stdio is null).
    Default,
    /// A console of its own, visible (`CREATE_NEW_CONSOLE`): for a program that is a server in a
    /// console and that the person ends by closing that window.
    NewConsole,
}

/// How a program is started.
#[derive(Debug, Clone, Copy)]
pub struct LaunchOpts<'a> {
    /// The working folder of the child, absolute and existing.
    pub cwd: &'a Path,
    pub window: Window,
}

/// `CREATE_NEW_CONSOLE`.
#[cfg(windows)]
const NEW_CONSOLE: u32 = 0x0000_0010;
/// `CREATE_NEW_PROCESS_GROUP`: a Ctrl-C of this console does not reach the child.
#[cfg(windows)]
const NEW_PROCESS_GROUP: u32 = 0x0000_0200;

/// The Windows creation flags of a start. Always its own process group; never
/// `CREATE_NO_WINDOW` and never `DETACHED_PROCESS` (an invisible process is what an endpoint
/// protection looks for).
#[cfg(windows)]
#[must_use]
pub fn creation_flags(window: Window) -> u32 {
    match window {
        Window::Default => NEW_PROCESS_GROUP,
        Window::NewConsole => NEW_PROCESS_GROUP | NEW_CONSOLE,
    }
}

// Absolute, on this machine (no network share, no device namespace) and without a `..`. The
// caller has checked its paths already; this is the second layer, so that a program on a share
// or one reached by climbing out of a root never starts, whatever the caller did.
use crate::child::local_absolute;

/// Start `program` with `args` and return its process id. The call does not wait for the
/// program; a thread of this process collects its exit status so that it is not left as a zombie.
///
/// # Errors
///
/// The program or the working folder is not absolute or does not exist, an argument holds a NUL,
/// or the system refused to start it.
pub fn spawn(program: &Path, args: &[&str], opts: &LaunchOpts<'_>) -> Result<u32, String> {
    spawn_with(program, args, opts, &[])
}

/// [`spawn`] with variables set for the child in addition to the environment of this process
/// (which it inherits as with [`spawn`]): a starter that tells the program something, such as
/// hub-desktop telling a hub it starts with itself to stay (`<APP>_STAY`). A name is upper case
/// letters, digits and `_`; a value holds no NUL.
///
/// # Errors
///
/// As [`spawn`], or a name or a value that is not one.
pub fn spawn_with(
    program: &Path,
    args: &[&str],
    opts: &LaunchOpts<'_>,
    set: &[(&str, &str)],
) -> Result<u32, String> {
    if let Some((name, _)) = set
        .iter()
        .find(|(name, value)| !plain_var(name) || value.contains('\0'))
    {
        return Err(format!(
            "not started: {name:?} is not a variable that can be set"
        ));
    }
    if !local_absolute(program) || !program.is_file() {
        return Err(format!(
            "not started: {} is not an absolute path to a file",
            program.display()
        ));
    }
    if !local_absolute(opts.cwd) || !opts.cwd.is_dir() {
        return Err(format!(
            "not started: {} is not an absolute path to a folder",
            opts.cwd.display()
        ));
    }
    if args.iter().any(|a| a.contains('\0')) {
        return Err("not started: an argument holds a NUL".to_owned());
    }
    #[cfg(windows)]
    crate::handles::keep_standard_handles();
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(opts.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for (name, value) in set {
        command.env(name, value);
    }
    apply(&mut command, opts.window);
    let mut child = command
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", program.display()))?;
    let pid = child.id();
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(pid)
}

/// A name of a variable: upper case letters, digits and `_`, not starting with a digit.
fn plain_var(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

#[cfg(windows)]
fn apply(command: &mut Command, window: Window) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(creation_flags(window));
}

#[cfg(unix)]
fn apply(command: &mut Command, _window: Window) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(not(any(windows, unix)))]
fn apply(_command: &mut Command, _window: Window) {}

#[cfg(test)]
mod tests {
    use super::*;
    use appkit_scratch::Scratch;

    fn opts(cwd: &Path) -> LaunchOpts<'_> {
        LaunchOpts {
            cwd,
            window: Window::Default,
        }
    }

    #[test]
    fn a_relative_program_is_refused() {
        let dir = Scratch::new("launch-rel");
        let err = spawn(Path::new("tasks-hub"), &[], &opts(&dir)).unwrap_err();
        assert!(err.contains("absolute"), "{err}");
    }

    #[test]
    fn a_missing_program_is_refused() {
        let dir = Scratch::new("launch-missing");
        let err = spawn(&dir.join("nope.exe"), &[], &opts(&dir)).unwrap_err();
        assert!(err.contains("not started"), "{err}");
    }

    #[test]
    fn a_folder_is_no_program() {
        let dir = Scratch::new("launch-dir");
        let err = spawn(&dir, &[], &opts(&dir)).unwrap_err();
        assert!(err.contains("not started"), "{err}");
    }

    #[test]
    fn a_relative_or_missing_working_folder_is_refused() {
        let dir = Scratch::new("launch-cwd");
        let exe = std::env::current_exe().unwrap();
        assert!(spawn(&exe, &[], &opts(Path::new("."))).is_err());
        assert!(spawn(&exe, &[], &opts(&dir.join("gone"))).is_err());
    }

    #[test]
    fn a_path_that_climbs_is_refused_as_program_and_as_folder() {
        let dir = Scratch::new("launch-climb");
        let exe = std::env::current_exe().unwrap();
        let climbing = dir.join("..").join(exe.file_name().unwrap());
        assert!(!local_absolute(&climbing));
        assert!(spawn(&climbing, &[], &opts(&dir)).is_err());
        assert!(spawn(&exe, &[], &opts(&dir.join(".."))).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn a_program_on_a_share_or_in_the_device_namespace_is_refused() {
        for path in [
            r"\\server\share\tool.exe",
            r"\\?\UNC\server\share\tool.exe",
            r"\\.\pipe\tool",
        ] {
            assert!(!local_absolute(Path::new(path)), "{path}");
        }
        assert!(local_absolute(Path::new(r"C:\Program Files\app\app.exe")));
        assert!(local_absolute(Path::new(
            r"\\?\C:\Program Files\app\app.exe"
        )));
    }

    #[test]
    fn a_nul_in_an_argument_is_refused() {
        let dir = Scratch::new("launch-nul");
        let exe = std::env::current_exe().unwrap();
        let err = spawn(&exe, &["a\0b"], &opts(&dir)).unwrap_err();
        assert!(err.contains("NUL"), "{err}");
    }

    #[cfg(windows)]
    #[test]
    fn the_flags_never_hide_a_window() {
        const NO_WINDOW: u32 = 0x0800_0000;
        const DETACHED: u32 = 0x0000_0008;
        for window in [Window::Default, Window::NewConsole] {
            let flags = creation_flags(window);
            assert_eq!(flags & (NO_WINDOW | DETACHED), 0, "{window:?}");
            assert_ne!(flags & NEW_PROCESS_GROUP, 0, "{window:?}");
        }
        assert_eq!(creation_flags(Window::Default) & NEW_CONSOLE, 0);
        assert_ne!(creation_flags(Window::NewConsole) & NEW_CONSOLE, 0);
    }
}
