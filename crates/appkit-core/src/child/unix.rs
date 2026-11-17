//! The Unix half of `child`: the process group, its watcher, and on Linux the parent-death
//! signal.
//!
//! The watcher ends the group when the app is gone without ending it (killed, crashed): it is a
//! process of the group, forked from the child before the child's program is loaded, that does
//! nothing but wait on a pipe whose only write end the app holds ([`Guard`]). When the app ends,
//! however it ends, the system closes that end; the watcher reads end of file and kills its
//! group, itself with it. A byte on the pipe instead lets it end alone (the group is the
//! person's then, [`Guard::release`]). While the watcher lives, the group's id cannot be given to
//! anything else. No program is started for it and no shell: it is the same fork that becomes
//! the child, so Linux and macOS behave alike.
#![allow(unsafe_code)]

use std::io::{self, Write};
use std::os::fd::{AsRawFd, RawFd};

use tokio::process::{Child, Command};

/// The most file descriptors the watcher closes. Above it a descriptor the app had open would
/// stay open in the watcher (which only matters for the end of file of a pipe).
const MOST_DESCRIPTORS: libc::c_int = 65_536;

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
/// waits for), and on Linux the system also ends the child itself then.
///
/// With `memory` the child's address space is limited to that many bytes (Linux, `RLIMIT_AS`);
/// a limit that cannot be set keeps the child from starting, and so does a watcher that cannot
/// be started.
///
/// # Errors
///
/// The pipe of the watcher could not be made.
pub(super) fn prepare(command: &mut Command, memory: Option<u64>) -> io::Result<Guard> {
    command.process_group(0);
    let (reader, writer) = io::pipe()?;
    let highest = highest_descriptor();
    #[cfg(not(target_os = "linux"))]
    let _ = memory;
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
    // calls only `getpid`, `fork`, `waitpid`, `close`, `read`, `kill` and `_exit` (all
    // async-signal-safe), takes no lock and does not allocate.
    unsafe {
        command.pre_exec(move || start_watcher(reader.as_raw_fd(), highest));
    }
    Ok(Guard(writer))
}

/// The number of descriptors the watcher closes: the app's limit, at most [`MOST_DESCRIPTORS`].
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
unsafe fn start_watcher(pipe: RawFd, highest: libc::c_int) -> io::Result<()> {
    // SAFETY: plain system calls; the branches of the forks end with `_exit` and never return.
    unsafe {
        let group = libc::getpid();
        let middle = libc::fork();
        if middle == -1 {
            return Err(io::Error::last_os_error());
        }
        if middle == 0 {
            match libc::fork() {
                0 => watch(pipe, group, highest),
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

/// The watcher: close everything of the app but the pipe, wait for its end, then end the group
/// (end of file, or an error) or end alone (a byte: the group was released).
///
/// # Safety
///
/// Only in the forked watcher.
unsafe fn watch(pipe: RawFd, group: libc::pid_t, highest: libc::c_int) -> ! {
    // SAFETY: plain system calls on numbers and a byte on the stack; `_exit` ends the process.
    unsafe {
        // The app's other descriptors (the pipes of other children, its sockets) are not the
        // watcher's: holding them would keep another child from seeing the end of its input.
        for fd in 0..highest {
            if fd != pipe {
                libc::close(fd);
            }
        }
        let mut byte = 0u8;
        let read = loop {
            let n = libc::read(pipe, (&raw mut byte).cast(), 1);
            if n == -1 && io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            break n;
        };
        if read != 1 {
            libc::kill(-group, libc::SIGKILL);
        }
        libc::_exit(0)
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
