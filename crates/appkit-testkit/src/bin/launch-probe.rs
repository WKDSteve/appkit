//! A program to be started by `appkit_hub::launch::spawn` in a test: it writes down what it was
//! given, so the test can check the start from the child's side.
//!
//! `launch-probe <out> [args...]` writes `<out>` (a JSON file, made whole in one rename) with the
//! arguments after `<out>`, the working folder, what each standard stream is (`null`, `other` or
//! `none`), how many processes share its console (Windows) and, for each `--probe-handle=<n>`
//! argument, whether a handle with that number is open in this process and which file it is
//! (Windows). `open` alone says nothing about inheritance: a process holds most small numbers
//! itself, so whether a handle was inherited is a question for `file`.
#![allow(unsafe_code)]

use std::path::Path;

use serde_json::{json, Value};

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args.next().expect("the first argument is the output file");
    let rest: Vec<String> = args.collect();
    let handles: Vec<Value> = rest
        .iter()
        .filter_map(|a| a.strip_prefix("--probe-handle="))
        .map(|n| {
            let value = n.parse().unwrap_or(0);
            json!({ "handle": n, "open": handle_is_open(value), "file": handle_file(value) })
        })
        .collect();
    let doc = json!({
        "args": rest,
        "cwd": std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default(),
        "stdio": stdio(),
        "console_processes": console_processes(),
        "handles": handles,
    });
    write_whole(Path::new(&out), &doc.to_string());
}

fn write_whole(path: &Path, text: &str) {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text).expect("write the probe file");
    std::fs::rename(&tmp, path).expect("make the probe file whole");
}

#[cfg(windows)]
mod sys {
    use windows_sys::Win32::Foundation::{GetHandleInformation, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileType, GetFinalPathNameByHandleW, FILE_NAME_NORMALIZED, FILE_TYPE_CHAR,
        FILE_TYPE_DISK, VOLUME_NAME_DOS,
    };
    use windows_sys::Win32::System::Console::{
        GetConsoleMode, GetConsoleProcessList, GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE,
        STD_OUTPUT_HANDLE,
    };

    pub fn stdio() -> Vec<&'static str> {
        [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE]
            .into_iter()
            .map(|which| {
                // SAFETY: GetStdHandle only reads the standard handle of this process.
                let h = unsafe { GetStdHandle(which) };
                if h.is_null() || h == INVALID_HANDLE_VALUE {
                    return "none";
                }
                let mut mode = 0u32;
                // SAFETY: `h` is a handle value of this process and `mode` a valid place.
                let console = unsafe { GetConsoleMode(h, &raw mut mode) } != 0;
                // SAFETY: as above; the call only asks the type of the handle.
                let character_device = unsafe { GetFileType(h) } == FILE_TYPE_CHAR;
                if character_device && !console {
                    "null"
                } else {
                    "other"
                }
            })
            .collect()
    }

    pub fn console_processes() -> u32 {
        let mut list = [0u32; 16];
        // SAFETY: `list` is a valid place for the 16 ids the call may write.
        unsafe { GetConsoleProcessList(list.as_mut_ptr(), 16) }
    }

    pub fn handle_is_open(value: usize) -> bool {
        let mut flags = 0u32;
        // SAFETY: the call only asks about a handle value of this process; a number that is not
        // one makes it fail, it reads and writes nothing else.
        unsafe { GetHandleInformation(value as HANDLE, &raw mut flags) != 0 }
    }

    /// The path of the file behind a handle number, in the spelling of `std::fs::canonicalize`.
    /// None for a number that is no handle and for any handle that is not a file on a disk: asking
    /// a pipe for its path can block.
    pub fn handle_file(value: usize) -> Option<String> {
        const CAPACITY: u32 = 1024;
        let handle = value as HANDLE;
        // SAFETY: the call only asks the type; a number that is not a handle gives "unknown".
        if unsafe { GetFileType(handle) } != FILE_TYPE_DISK {
            return None;
        }
        let mut path = [0u16; CAPACITY as usize];
        // SAFETY: `path` is a valid place for the CAPACITY characters the call may write.
        let length = unsafe {
            GetFinalPathNameByHandleW(
                handle,
                path.as_mut_ptr(),
                CAPACITY,
                FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
            )
        };
        // 0 is a failure, and a length of CAPACITY or more is a path that did not fit.
        (length > 0 && length < CAPACITY)
            .then(|| String::from_utf16_lossy(&path[..length as usize]))
    }
}

#[cfg(windows)]
fn stdio() -> Vec<&'static str> {
    sys::stdio()
}
#[cfg(windows)]
fn console_processes() -> u32 {
    sys::console_processes()
}
#[cfg(windows)]
fn handle_is_open(value: usize) -> bool {
    sys::handle_is_open(value)
}
#[cfg(windows)]
fn handle_file(value: usize) -> Option<String> {
    sys::handle_file(value)
}

#[cfg(not(windows))]
fn stdio() -> Vec<&'static str> {
    Vec::new()
}
#[cfg(not(windows))]
fn console_processes() -> u32 {
    0
}
#[cfg(not(windows))]
fn handle_is_open(_value: usize) -> bool {
    false
}
#[cfg(not(windows))]
fn handle_file(_value: usize) -> Option<String> {
    None
}
