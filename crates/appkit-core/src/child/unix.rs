//! The Unix half of `child`: the process group, its watcher, and on Linux the parent-death
//! signal.
//!
//! The watcher ends the group when the app is gone without ending it (killed, crashed). It is a
//! process of the group that does nothing but wait on a pipe whose only write end the app holds
//! ([`Guard`]). When the app ends, however it ends, the system closes that end; the watcher reads
//! end of file and kills its group, itself with it. A byte on the pipe instead lets it end alone
//! (the group is the person's then, [`Guard::release`]). While the watcher lives, the group's id
//! cannot be given to anything else. It ignores the signals a group is told to stop with
//! (`kill 0` of a child), so only the app or a `SIGKILL` ends it.
//!
//! How it comes to be: the child forks it before the child's program is loaded, twice, so it is
//! nobody's child there. When the app called `watcher_main` first in its `main`, the watcher then
//! loads the app's own program (`execve`, argument `--appkit-child-watcher`, the pipe on
//! descriptor 3, the standard streams on `/dev/null`, no environment): it keeps no copy of the
//! app's memory, and every other descriptor of the app is closed by the system (close-on-exec).
//! Without that call (a test program) Linux keeps the fork as the watcher: it closes the app's
//! descriptors itself and holds a copy of the memory the app changes afterwards; macOS then gets
//! no watcher at all, because closing every descriptor there can hit a guarded one, which ends
//! the process.
#![allow(unsafe_code)]

use std::ffi::{CStr, CString, OsStr};
use std::io::{self, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::sync::OnceLock;

use tokio::process::{Child, Command};

/// The argument that makes the app's program a watcher.
pub(super) const WATCHER_ARG: &str = "--appkit-child-watcher";
const WATCHER_ARG_C: &CStr = c"--appkit-child-watcher";

/// The descriptor the watcher program reads its pipe on.
const WATCHER_FD: RawFd = 3;

/// The most file descriptors a forked watcher closes one by one (Linux without `close_range`).
/// Above it a descriptor the app had open would stay open in it.
const MOST_DESCRIPTORS: libc::c_int = 65_536;

/// The program a watcher loads: set by [`watcher_main`].
static PROGRAM: OnceLock<CString> = OnceLock::new();

/// See `child::watcher_main`.
pub(super) fn watcher_main() {
    if std::env::args_os().nth(1).as_deref() == Some(OsStr::new(WATCHER_ARG)) {
        serve();
    }
    // Linux: the running program itself, also when its file was replaced meanwhile.
    #[cfg(target_os = "linux")]
    let program = Some(c"/proc/self/exe".to_owned());
    #[cfg(not(target_os = "linux"))]
    let program = std::env::current_exe().ok().and_then(|path| {
        use std::os::unix::ffi::OsStrExt;
        CString::new(path.as_os_str().as_bytes()).ok()
    });
    if let Some(program) = program {
        let _ = PROGRAM.set(program);
    }
}

/// The program started as a watcher: wait, end the group or not, and exit.
fn serve() -> ! {
    // SAFETY: plain system calls on numbers and a byte on the stack.
    unsafe {
        ignore_stop_signals();
        wait_and_end(WATCHER_FD);
    }
    std::process::exit(0)
}

/// The write end of the pipe the watcher of a child's group waits on. Dropping it (or the app
/// ending) makes the watcher end the group.
#[derive(Debug)]
pub(super) struct Guard(io::PipeWriter);

impl Guard {
    /// Let the group live on: the watcher reads a byte and ends without a signal.
    pub(super) fn release(mut self) {
        let _ = self.0.write_all(&[1]);
    }
}

/// The child leads a process group of its own (so the group can be ended as one), a watcher
/// in it ends the group when this process is gone (the returned [`Guard`] is the one end it
/// waits for; `None` on macOS without [`watcher_main`]), and on Linux the system also ends the
/// child itself then.
///
/// With `memory` the child's address space is limited to that many bytes (Linux, `RLIMIT_AS`);
/// a limit that cannot be set keeps the child from starting, and so does a watcher that cannot
/// be forked.
///
/// # Errors
///
/// The pipe of the watcher could not be made.
pub(super) fn prepare(command: &mut Command, memory: Option<u64>) -> io::Result<Option<Guard>> {
    command.process_group(0);
    let program = PROGRAM.get().cloned();
    #[cfg(not(target_os = "linux"))]
    {
        let _ = memory;
        if program.is_none() {
            return Ok(None);
        }
    }
    // Close-on-exec on both ends: the child's program never holds them. macOS has no `pipe2`, so
    // there std sets the flag right after `pipe`; a child another thread forks in between keeps
    // the write end until it ends, and the group of this child then waits for that.
    let (reader, writer) = io::pipe()?;
    let highest = highest_descriptor();
    #[cfg(target_os = "linux")]
    {
        let app = libc::pid_t::try_from(std::process::id()).unwrap_or(0);
        // SAFETY: the closure runs in the forked child before the program is loaded and calls
        // only `setrlimit`, `prctl` and `getppid` (all async-signal-safe); it takes no lock and
        // does not allocate (the errors are made from numbers).
        unsafe {
            command.pre_exec(move || {
                if let Some(bytes) = memory {
                    // `rlim_t` is 64 bits wide on most targets and 32 on some.
                    #[allow(clippy::useless_conversion)]
                    let bytes = libc::rlim_t::try_from(bytes).unwrap_or(libc::RLIM_INFINITY);
                    let limit = libc::rlimit {
                        rlim_cur: bytes,
                        rlim_max: bytes,
                    };
                    if libc::setrlimit(libc::RLIMIT_AS, &limit) != 0 {
                        return Err(io::Error::last_os_error());
                    }
                }
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                    return Err(io::Error::last_os_error());
                }
                // The app was gone before the signal was asked for: nobody would send it.
                if libc::getppid() != app {
                    return Err(io::Error::from_raw_os_error(libc::ESRCH));
                }
                Ok(())
            });
        }
    }
    // After the closure above: the getppid check comes first, and a watcher is forked only for a
    // child that will start. The reader moves into the closure, so it lives until the fork.
    // SAFETY: the closure runs in the forked child before the program is loaded; `start_watcher`
    // calls only async-signal-safe functions of the system (`fork`, `waitpid`, `signal`, `fcntl`,
    // `dup2`, `open`, `execve`, `close`, `read`, `getpgrp`, `getpid`, `kill`, `_exit`, and the
    // `close_range` system call), takes no lock and does not allocate.
    unsafe {
        command.pre_exec(move || start_watcher(reader.as_raw_fd(), highest, program.as_deref()));
    }
    Ok(Some(Guard(writer)))
}

/// The number of descriptors a forked watcher closes one by one: the app's limit, at most
/// [`MOST_DESCRIPTORS`].
fn highest_descriptor() -> libc::c_int {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `limit` is a live struct of the type the call writes.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut limit) } != 0 {
        return MOST_DESCRIPTORS;
    }
    libc::c_int::try_from(limit.rlim_cur)
        .unwrap_or(MOST_DESCRIPTORS)
        .min(MOST_DESCRIPTORS)
}

/// In the child before its program is loaded: fork the watcher of the group this process
/// leads. Twice, so the watcher is nobody's child here (the child's program never sees an
/// unknown child of its own) and the child waits only for the short middle process.
///
/// # Safety
///
/// Only between fork and exec, with `pipe` the read end of the watcher's pipe.
unsafe fn start_watcher(
    pipe: RawFd,
    highest: libc::c_int,
    program: Option<&CStr>,
) -> io::Result<()> {
    // SAFETY: plain system calls; the branches of the forks end with `_exit` and never return.
    unsafe {
        let middle = libc::fork();
        if middle == -1 {
            return Err(io::Error::last_os_error());
        }
        if middle == 0 {
            match libc::fork() {
                0 => become_watcher(pipe, highest, program),
                -1 => libc::_exit(1),
                _ => libc::_exit(0),
            }
        }
        let mut status = 0;
        while libc::waitpid(middle, &raw mut status, 0) == -1 {
            match io::Error::last_os_error().raw_os_error() {
                Some(libc::EINTR) => {}
                // An app that ignores SIGCHLD has its children reaped by the system: the middle
                // process is gone, and whether it forked the watcher cannot be asked any more.
                Some(libc::ECHILD) => return Ok(()),
                _ => return Err(io::Error::last_os_error()),
            }
        }
        if !libc::WIFEXITED(status) || libc::WEXITSTATUS(status) != 0 {
            // No watcher: the child does not start (`EAGAIN`, as a failed fork says).
            return Err(io::Error::from_raw_os_error(libc::EAGAIN));
        }
    }
    Ok(())
}

/// The forked watcher: load the app's program as the watcher, or (Linux, without a program or
/// when loading it failed) be the watcher here.
///
/// # Safety
///
/// Only in the forked watcher.
unsafe fn become_watcher(pipe: RawFd, highest: libc::c_int, program: Option<&CStr>) -> ! {
    // SAFETY: plain system calls on numbers, on pointers to constant strings and to arrays on the
    // stack; `_exit` ends the process.
    unsafe {
        ignore_stop_signals();
        if let Some(program) = program {
            // The pipe on its fixed descriptor and kept over the exec (`dup2` clears close-on-exec
            // on the copy); the standard streams (the child's pipes to the app) on /dev/null.
            // Everything else of the app is close-on-exec and goes with the exec.
            let moved = if pipe == WATCHER_FD {
                libc::fcntl(pipe, libc::F_SETFD, 0) != -1
            } else {
                libc::dup2(pipe, WATCHER_FD) != -1
            };
            let null = libc::open(c"/dev/null".as_ptr(), libc::O_RDWR | libc::O_CLOEXEC);
            if moved && null != -1 && (0..3).all(|fd| libc::dup2(null, fd) != -1) {
                let argv = [program.as_ptr(), WATCHER_ARG_C.as_ptr(), std::ptr::null()];
                let envp = [std::ptr::null::<libc::c_char>()];
                libc::execve(program.as_ptr(), argv.as_ptr(), envp.as_ptr());
            }
        }
        // Not loaded: Linux keeps the fork as the watcher; macOS has none (see the module).
        #[cfg(target_os = "linux")]
        {
            close_all_but(pipe, highest);
            wait_and_end(pipe);
        }
        #[cfg(not(target_os = "linux"))]
        let _ = highest;
        libc::_exit(0)
    }
}

/// Close every descriptor but `keep`: the app's other descriptors (the pipes of other children,
/// its sockets) are not a forked watcher's, and holding them would keep another child from
/// seeing the end of its input.
///
/// # Safety
///
/// Only in the forked watcher.
#[cfg(target_os = "linux")]
unsafe fn close_all_but(keep: RawFd, highest: libc::c_int) {
    // SAFETY: plain system calls on numbers.
    unsafe {
        let Ok(at) = libc::c_uint::try_from(keep) else {
            return;
        };
        // `close_range` (Linux 5.9): two calls instead of one per descriptor. `keep` is at least
        // 3, the standard streams are taken.
        if at > 0
            && libc::syscall(libc::SYS_close_range, 0, at - 1, 0) == 0
            && libc::syscall(libc::SYS_close_range, at + 1, libc::c_uint::MAX, 0) == 0
        {
            return;
        }
        for fd in 0..highest {
            if fd != keep {
                libc::close(fd);
            }
        }
    }
}

/// The signals a group is told to stop with: a child's `kill 0` must not end its watcher.
///
/// # Safety
///
/// Only in a watcher.
unsafe fn ignore_stop_signals() {
    for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP, libc::SIGQUIT] {
        // SAFETY: `SIG_IGN` for a signal that may be ignored.
        unsafe { libc::signal(signal, libc::SIG_IGN) };
    }
}

/// Wait on the pipe until the app writes a byte (the group was released: end alone) or its end
/// is closed (the app is gone: kill the group). Nothing on a descriptor that is no pipe at all
/// (the program was started by hand with the argument), and never the group of a terminal job
/// the watcher leads itself.
///
/// # Safety
///
/// Only in a watcher.
unsafe fn wait_and_end(pipe: RawFd) {
    // SAFETY: plain system calls on numbers and a byte on the stack.
    unsafe {
        let mut byte = 0u8;
        let read = loop {
            let n = libc::read(pipe, (&raw mut byte).cast(), 1);
            if n == -1 && io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            break n;
        };
        let no_pipe = read == -1 && io::Error::last_os_error().raw_os_error() == Some(libc::EBADF);
        if read == 1 || no_pipe {
            return;
        }
        let group = libc::getpgrp();
        if group > 1 && group != libc::getpid() {
            libc::kill(-group, libc::SIGKILL);
        }
    }
}

/// Start the child.
///
/// Linux sends the parent-death signal when the *thread* that started the child ends, not the
/// process. A thread of a blocking pool ends after a pause, and its children would end with it.
/// So one thread that lives as long as the process starts every child.
#[cfg(target_os = "linux")]
pub(super) fn spawn(command: Command) -> io::Result<Child> {
    use std::sync::mpsc::{sync_channel, Sender, SyncSender};
    use std::sync::{Mutex, OnceLock, PoisonError};

    use tokio::runtime::Handle;

    type Order = (Command, Handle, SyncSender<io::Result<Child>>);
    static STARTER: OnceLock<Option<Mutex<Sender<Order>>>> = OnceLock::new();

    // The child's pipes belong to the runtime of whoever asked.
    let handle = Handle::try_current().map_err(io::Error::other)?;
    let starter = STARTER.get_or_init(|| {
        let (orders, inbox) = std::sync::mpsc::channel::<Order>();
        std::thread::Builder::new()
            .name("appkit-child-starter".to_owned())
            .spawn(move || {
                for (mut command, handle, answer) in inbox {
                    let _runtime = handle.enter();
                    let _ = answer.send(command.spawn());
                }
            })
            .ok()
            .map(|_| Mutex::new(orders))
    });
    let Some(starter) = starter else {
        return Err(io::Error::other(
            "the thread that starts children could not be started",
        ));
    };
    let (answer, started) = sync_channel(1);
    starter
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .send((command, handle, answer))
        .map_err(|_| io::Error::other("the thread that starts children is gone"))?;
    started
        .recv()
        .map_err(|_| io::Error::other("the thread that starts children is gone"))?
}

#[cfg(not(target_os = "linux"))]
pub(super) fn spawn(mut command: Command) -> io::Result<Child> {
    command.spawn()
}

/// End every process of the group `group` leads.
pub(super) fn kill_group(group: u32) {
    let Ok(group) = libc::pid_t::try_from(group) else {
        return;
    };
    // 0 and 1 would name this process's own group and every process.
    if group > 1 {
        // SAFETY: a plain system call with numbers; a group that is gone is an error that is
        // ignored.
        unsafe { libc::kill(-group, libc::SIGKILL) };
    }
}
