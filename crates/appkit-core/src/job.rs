//! A Windows job object that ends a child together with everything it started.
//!
//! `Child::start_kill` ends the one process. A program that started others (an engine that
//! runs a shell, a test runner) leaves them running when the watchdog fires. The child is
//! put into a job with `KILL_ON_JOB_CLOSE`; dropping the [`Job`] closes the handle, and the
//! system ends every process still in it. On other systems the type is empty and does nothing.
#![allow(unsafe_code)]

/// Ends the process tree of a child when it is dropped (Windows), else nothing.
#[derive(Debug)]
pub struct Job {
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
}

// SAFETY: the handle is a plain kernel handle; closing it from another thread is allowed.
#[cfg(windows)]
unsafe impl Send for Job {}
// SAFETY: as above; the type has no methods that use the handle through `&self`.
#[cfg(windows)]
unsafe impl Sync for Job {}

impl Job {
    /// Put `child` into a new kill-on-close job. `None` when the system refuses (the child
    /// then runs as before: only the process itself is ended by a kill).
    #[cfg(windows)]
    #[must_use]
    pub fn adopt(child: &tokio::process::Child) -> Option<Self> {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };

        let process = child.raw_handle()?;
        // SAFETY: a null attribute and name are documented as valid; the result is checked.
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return None;
        }
        // SAFETY: an all-zero struct is a valid "no limits" value of this plain-data type.
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let size = u32::try_from(std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
            .unwrap_or(u32::MAX);
        // SAFETY: `job` is a job handle we own, `info` is a live struct of the size given.
        let set = unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                std::ptr::addr_of!(info).cast(),
                size,
            )
        };
        // SAFETY: `job` is ours; `process` is the live handle of the child (it is not reaped
        // yet, the caller holds the `Child`).
        let assigned = set != 0 && unsafe { AssignProcessToJobObject(job, process) } != 0;
        if !assigned {
            // SAFETY: `job` is ours and not used again.
            unsafe { CloseHandle(job) };
            return None;
        }
        Some(Self { handle: job })
    }

    /// No job on this system.
    #[cfg(not(windows))]
    #[must_use]
    pub fn adopt(_child: &tokio::process::Child) -> Option<Self> {
        None
    }
}

#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: the handle is ours and closed once; closing ends the processes in the job.
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.handle) };
    }
}
