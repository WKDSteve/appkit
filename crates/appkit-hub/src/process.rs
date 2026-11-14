//! One other process, by its id: is it still there, and end it.
//!
//! `<program> stop` ends the hub that `hub.json` names. It does that with a call of the system,
//! not with a program (`taskkill.exe`, `/bin/kill`). On Windows the process is opened first and
//! the handle is kept: while it is open the system gives the id to no other process, so what is
//! ended is the process that was opened, whatever happened in between.
#![allow(unsafe_code)]

/// A process of this machine that is not a child of this one.
#[derive(Debug)]
pub(crate) struct Process {
    pid: u32,
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
}

// SAFETY: the handle is a plain kernel handle; waiting on it, ending its process and closing
// it are allowed from any thread.
#[cfg(windows)]
unsafe impl Send for Process {}
// SAFETY: as above; no method changes the handle.
#[cfg(windows)]
unsafe impl Sync for Process {}

/// An id that names one process. 0 is nobody on Windows and the caller's own group for `kill`;
/// an id that does not fit a signed number would name a group there too.
fn names_one_process(pid: u32) -> bool {
    pid != 0 && i32::try_from(pid).is_ok() && pid != std::process::id()
}

impl Process {
    /// The id this was opened with.
    pub(crate) fn pid(&self) -> u32 {
        self.pid
    }
}

#[cfg(windows)]
impl Process {
    /// Open the process `pid` to watch and to end it.
    ///
    /// # Errors
    ///
    /// The id names no process (none, or this one), or the system refuses (a process of
    /// another user).
    pub(crate) fn open(pid: u32) -> Result<Self, String> {
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
        };
        if !names_one_process(pid) {
            return Err(format!("{pid} is not the id of another process"));
        }
        // SAFETY: plain values; the handle is not inheritable and is checked.
        let handle = unsafe { OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, 0, pid) };
        if handle.is_null() {
            return Err(format!(
                "the process {pid} cannot be opened: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(Self { pid, handle })
    }

    /// Whether the process has ended.
    pub(crate) fn is_gone(&self) -> bool {
        use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
        use windows_sys::Win32::System::Threading::WaitForSingleObject;
        // SAFETY: the handle is ours and open; a wait of no time only asks.
        unsafe { WaitForSingleObject(self.handle, 0) == WAIT_OBJECT_0 }
    }

    /// End the process now. One that has ended meanwhile is no error.
    ///
    /// # Errors
    ///
    /// The system refused.
    pub(crate) fn end(&self) -> Result<(), String> {
        use windows_sys::Win32::System::Threading::TerminateProcess;
        // SAFETY: the handle is ours and open, with the right to end the process.
        if unsafe { TerminateProcess(self.handle, 1) } != 0 || self.is_gone() {
            Ok(())
        } else {
            Err(format!(
                "the process {} could not be stopped: {}",
                self.pid,
                std::io::Error::last_os_error()
            ))
        }
    }
}

#[cfg(windows)]
impl Drop for Process {
    fn drop(&mut self) {
        // SAFETY: the handle is ours and closed once.
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.handle) };
    }
}

#[cfg(unix)]
impl Process {
    /// The signal 0 asks whether the process is there and may be signalled; nothing is sent.
    fn signal(&self, signal: libc::c_int) -> std::io::Result<()> {
        // `names_one_process` made sure the id fits and is no group.
        let pid = libc::pid_t::try_from(self.pid).unwrap_or(libc::pid_t::MAX);
        // SAFETY: `kill` with the id of one process and a valid signal number.
        if unsafe { libc::kill(pid, signal) } == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }

    /// Name the process `pid` to watch and to end it.
    ///
    /// # Errors
    ///
    /// The id names no process (none, or this one), or the system refuses (a process of
    /// another user).
    pub(crate) fn open(pid: u32) -> Result<Self, String> {
        if !names_one_process(pid) {
            return Err(format!("{pid} is not the id of another process"));
        }
        let process = Self { pid };
        process
            .signal(0)
            .map_err(|e| format!("the process {pid} cannot be opened: {e}"))?;
        Ok(process)
    }

    /// Whether the process has ended: no process has the id any more, or it is a child of this
    /// process that has ended and was not waited for yet (it keeps its id until then, and a
    /// signal still reaches it). The child is only looked at, not waited for: its exit status
    /// stays for whoever holds it.
    pub(crate) fn is_gone(&self) -> bool {
        if matches!(self.signal(0), Err(e) if e.raw_os_error() == Some(libc::ESRCH)) {
            return true;
        }
        // SAFETY: an all-zero `siginfo_t` is a valid value to be filled in; `waitid` with
        // `WNOHANG | WNOWAIT` neither blocks nor takes the exit status away. It fails for a
        // process that is not a child of this one, and leaves the id 0 for one that still runs.
        unsafe {
            let mut info: libc::siginfo_t = std::mem::zeroed();
            libc::waitid(
                libc::P_PID,
                libc::id_t::from(self.pid),
                &raw mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            ) == 0
                && info.si_pid() != 0
        }
    }

    /// Ask the process to end (`SIGTERM`). One that has ended meanwhile is no error.
    ///
    /// # Errors
    ///
    /// The system refused.
    pub(crate) fn end(&self) -> Result<(), String> {
        match self.signal(libc::SIGTERM) {
            Ok(()) => Ok(()),
            Err(e) if e.raw_os_error() == Some(libc::ESRCH) => Ok(()),
            Err(e) => Err(format!(
                "the process {} could not be stopped: {e}",
                self.pid
            )),
        }
    }
}

#[cfg(not(any(windows, unix)))]
impl Process {
    pub(crate) fn open(pid: u32) -> Result<Self, String> {
        let _ = names_one_process(pid);
        Err("ending a process is not built for this system".to_owned())
    }

    pub(crate) fn is_gone(&self) -> bool {
        false
    }

    pub(crate) fn end(&self) -> Result<(), String> {
        Err("ending a process is not built for this system".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nobody_a_group_and_this_process_are_not_opened() {
        for pid in [0, u32::MAX, u32::MAX - 1, std::process::id()] {
            let err = Process::open(pid).unwrap_err();
            assert!(err.contains("not the id of another process"), "{err}");
        }
    }

    #[test]
    fn an_id_nobody_has_cannot_be_opened() {
        // Ids are multiples of four on Windows and stay far below this on Unix.
        let err = Process::open(0x7fff_fff1).unwrap_err();
        assert!(err.contains("cannot be opened"), "{err}");
    }
}
