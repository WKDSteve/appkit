//! The one place of the kit that calls the system directly (Windows).
//!
//! A program that Rust starts on Windows inherits every handle of this process that is marked
//! inheritable, whatever its own stdin, stdout and stderr are: the standard handles of a
//! process that was itself started with a pipe (`<program> start | ...`, a job of an editor)
//! are such handles. A hub started from there would hold the pipe of the caller open for as long
//! as it runs, and the caller would wait for the end of its output until the hub stops. The
//! stable standard library has no way to say "inherit nothing" (`inherit_handles` is unstable),
//! so the standard handles of this process are made not inheritable before the hub is started.
//! The handles stay open and usable here; only a program started later does not get them
//! (one that is given them as its stdio, `Stdio::inherit`, still does: the standard library
//! duplicates them for it).
//!
//! A handle is handed on down the whole chain: a child gets every inheritable handle of its
//! parent, also those that are not its stdio, and it cannot name those to keep them. So every
//! process on the way to the hub does this before it starts the next one (`start`, and
//! `install`, which runs `<program> start`).
//!
//! The script host (`wscript.exe`) did this before; an endpoint protection stops a program that
//! starts the script host (appkit/start-ueber-wscript-von-edr-beendet).
#![allow(unsafe_code)]

use std::os::windows::io::{AsHandle, AsRawHandle, BorrowedHandle};

use windows_sys::Win32::Foundation::{SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT};

/// Mark the standard handles of this process as not inheritable. A standard handle that is
/// missing (a program with no console) is skipped.
pub fn keep_standard_handles() {
    [
        std::io::stdin().as_handle(),
        std::io::stdout().as_handle(),
        std::io::stderr().as_handle(),
    ]
    .into_iter()
    .for_each(|h| {
        not_inheritable(h);
    });
}

/// Clear the inherit flag of `handle`. False when the system refuses it (no handle, or one that
/// is not a handle of a kernel object).
fn not_inheritable(handle: BorrowedHandle<'_>) -> bool {
    let raw = handle.as_raw_handle() as HANDLE;
    if raw.is_null() {
        return false;
    }
    // SAFETY: `raw` comes from a `BorrowedHandle` of the standard streams, so it is a handle of
    // this process that stays open for the call; the call changes only its inherit flag (mask
    // and value as documented for SetHandleInformation) and reads or writes no memory of ours.
    unsafe { SetHandleInformation(raw, HANDLE_FLAG_INHERIT, 0) != 0 }
}

/// Whether `handle` is marked inheritable (for the tests).
#[cfg(test)]
pub fn is_inheritable(handle: BorrowedHandle<'_>) -> bool {
    use windows_sys::Win32::Foundation::GetHandleInformation;
    let mut flags = 0u32;
    let raw = handle.as_raw_handle() as HANDLE;
    // SAFETY: as above; `flags` is a valid place for the one value the call writes.
    let ok = unsafe { GetHandleInformation(raw, &raw mut flags) } != 0;
    ok && flags & HANDLE_FLAG_INHERIT != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use appkit_scratch::Scratch;

    #[test]
    fn an_inheritable_handle_is_no_longer_inheritable_after_the_call() {
        // A file the standard library opens is not inheritable: it is made so here, as the
        // standard handles of a program started with a pipe are.
        let dir = Scratch::new("handles");
        let file = std::fs::File::create(dir.join("probe")).unwrap();
        let raw = file.as_handle();
        // SAFETY: the handle of `file`, open for the call.
        assert!(
            unsafe { SetHandleInformation(raw.as_raw_handle() as HANDLE, HANDLE_FLAG_INHERIT, 1) }
                != 0
        );
        assert!(is_inheritable(raw));
        assert!(not_inheritable(raw));
        assert!(!is_inheritable(raw));
        drop(file);
    }
}
