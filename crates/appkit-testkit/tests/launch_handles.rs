//! A handle of the hub does not reach a program it starts (Windows).
//!
//! One test in a file of its own: it replaces the standard output handle of the whole process for
//! a moment, which no other test of the same process may see.
#![allow(unsafe_code)]

#[cfg(windows)]
#[test]
fn a_standard_handle_of_the_hub_is_not_inherited_by_the_started_program() {
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use appkit_hub::launch::{spawn, LaunchOpts, Window};
    use appkit_scratch::Scratch;
    use windows_sys::Win32::Foundation::{SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT};
    use windows_sys::Win32::System::Console::{GetStdHandle, SetStdHandle, STD_OUTPUT_HANDLE};

    const PROBE: &str = env!("CARGO_BIN_EXE_launch-probe");

    /// Puts the standard output back when the test ends, also when an assertion stops it.
    struct RestoreStdout(HANDLE);
    impl Drop for RestoreStdout {
        fn drop(&mut self) {
            // SAFETY: puts the handle back that was there before.
            unsafe { SetStdHandle(STD_OUTPUT_HANDLE, self.0) };
        }
    }

    fn wait_for(out: &Path) -> serde_json::Value {
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if let Ok(text) = std::fs::read_to_string(out) {
                return serde_json::from_str(&text).unwrap();
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("the probe wrote nothing within 30 seconds");
    }

    let dir = Scratch::new("launch-handles");
    // The standard output of a hub that was started with a pipe: an inheritable handle.
    let file = std::fs::File::create(dir.join("stdout.txt")).unwrap();
    // The number alone proves nothing: the child holds most small numbers itself, so the probe says
    // which file is behind it.
    let path = std::fs::canonicalize(dir.join("stdout.txt")).unwrap();
    let path = path.to_str().unwrap();
    let raw = file.as_raw_handle();
    // SAFETY: the handle of `file`, open for the call; only its inherit flag changes.
    assert!(
        unsafe { SetHandleInformation(raw as _, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) } != 0
    );
    // SAFETY: reads the current standard output handle; the guard puts it back at the end.
    let _restore = RestoreStdout(unsafe { GetStdHandle(STD_OUTPUT_HANDLE) });
    // SAFETY: `raw` stays open longer than the guard, `file` was made first.
    assert!(unsafe { SetStdHandle(STD_OUTPUT_HANDLE, raw as _) } != 0);
    let number = raw as usize;
    let probe_arg = format!("--probe-handle={number}");

    // The control: a plain start passes the handle on, so that the probe can tell the difference.
    let control = dir.join("control.json");
    Command::new(PROBE)
        .arg(&control)
        .arg(&probe_arg)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    let seen = wait_for(&control);
    assert_eq!(
        seen["handles"][0]["file"].as_str(),
        Some(path),
        "the control start did not inherit the handle, so the test proves nothing: {seen}"
    );

    // The helper makes the standard handles not inheritable before it starts the program.
    let out = dir.join("seen.json");
    spawn(
        Path::new(PROBE),
        &[out.to_str().unwrap(), probe_arg.as_str()],
        &LaunchOpts {
            cwd: &dir,
            window: Window::Default,
        },
    )
    .unwrap();
    let seen = wait_for(&out);
    assert_ne!(seen["handles"][0]["file"].as_str(), Some(path), "{seen}");
}

#[cfg(not(windows))]
#[test]
fn a_standard_handle_of_the_hub_is_not_inherited_by_the_started_program() {
    println!("n/a  handle inheritance is a Windows matter");
}
