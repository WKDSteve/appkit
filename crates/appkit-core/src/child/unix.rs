//! The Unix half of `child`: the process group, and on Linux the parent-death signal.
#![allow(unsafe_code)]

use std::io;

use tokio::process::{Child, Command};

/// The child leads a process group of its own (so the group can be ended as one), and on Linux
/// the system ends it when this process is gone.
///
/// With `memory` the child's address space is limited to that many bytes (Linux, `RLIMIT_AS`);
/// a limit that cannot be set keeps the child from starting.
pub(super) fn prepare(command: &mut Command, memory: Option<u64>) {
    command.process_group(0);
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
