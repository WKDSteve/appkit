//! Open a page of a local hub in the default browser.
//!
//! Only `http://127.0.0.1:<port>/…` with a port the app names. Never `file://`, never a name
//! other than the loopback address, and never through the file association. On Windows the link
//! goes to `ShellExecuteW` (verb `open`), no helper program is started; elsewhere the program that
//! opens links gets exactly one argument, the link, and it is an absolute program.

#[cfg(not(windows))]
use std::ffi::OsString;
#[cfg(not(windows))]
use std::path::PathBuf;
#[cfg(not(windows))]
use std::process::{Command, Stdio};

/// The longest link that is opened.
const MAX_LEN: usize = 2048;

/// Is `url` a link `open` may pass on: the loopback address, a port of `ports`, and below it a
/// path, query and fragment of plain characters only? What a program could take as an option,
/// a path or a second argument (a space, a backslash, a quote, a control character, `@`) is not
/// plain.
#[must_use]
pub fn is_allowed(url: &str, ports: &[u16]) -> bool {
    if url.len() > MAX_LEN {
        return false;
    }
    let Some(rest) = url.strip_prefix("http://127.0.0.1:") else {
        return false;
    };
    let Some((port, tail)) = rest.split_once('/') else {
        return false;
    };
    // Digits only and no leading zero: "017615" is the same port but not the same text.
    let plain_port = !port.is_empty()
        && port.len() <= 5
        && port.bytes().all(|b| b.is_ascii_digit())
        && !port.starts_with('0');
    if !plain_port {
        return false;
    }
    let Ok(port) = port.parse::<u16>() else {
        return false;
    };
    ports.contains(&port) && tail.bytes().all(plain_byte)
}

fn plain_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || matches!(
            b,
            b'-' | b'.'
                | b'_'
                | b'~'
                | b'/'
                | b'%'
                | b'+'
                | b'='
                | b'&'
                | b'?'
                | b'#'
                | b':'
                | b','
        )
}

/// The program and arguments that open `url` (macOS and the rest; Windows asks the shell
/// directly, see [`shell_open`]).
#[cfg(not(windows))]
fn opener(url: &str) -> (PathBuf, Vec<OsString>) {
    if cfg!(target_os = "macos") {
        (PathBuf::from("/usr/bin/open"), vec![url.into()])
    } else {
        (PathBuf::from("/usr/bin/xdg-open"), vec![url.into()])
    }
}

/// Hand `url` to the shell (`ShellExecuteW`, verb `open`): no helper program is started, so no
/// `rundll32` (a known living-off-the-land binary that endpoint protection watches for).
#[cfg(windows)]
#[allow(unsafe_code)]
fn shell_open(url: &str) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let verb: Vec<u16> = "open\0".encode_utf16().collect();
    let target: Vec<u16> = std::ffi::OsStr::new(url)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: `verb` and `target` are NUL-terminated and live for the call; the other pointers
    // are documented as optional (null). The call opens the link and keeps no pointer.
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // Documented: a value above 32 is success, below it an error code.
    if result as usize > 32 {
        Ok(())
    } else {
        Err(format!(
            "could not start the browser: the shell answered {}",
            result as usize
        ))
    }
}

/// Open `url` in the default browser.
///
/// # Errors
///
/// The link is not allowed for `ports`, or the browser could not be started.
pub fn open(url: &str, ports: &[u16]) -> Result<(), String> {
    if !is_allowed(url, ports) {
        return Err("the link is not one this app opens: not opened".to_owned());
    }
    #[cfg(windows)]
    {
        crate::handles::keep_standard_handles();
        shell_open(url)
    }
    #[cfg(not(windows))]
    {
        let (program, args) = opener(url);
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("could not start the browser: {e}"))?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PORTS: &[u16] = &[17610, 17611, 8080];

    #[test]
    fn the_links_of_the_app_are_allowed() {
        for url in [
            "http://127.0.0.1:17610/#pair=0123456789abcdef",
            "http://127.0.0.1:17611/tasks-web/tasks.html",
            "http://127.0.0.1:8080/",
            "http://127.0.0.1:8080/a/b-c_d.e~f?x=1&y=2#frag",
        ] {
            assert!(is_allowed(url, PORTS), "{url}");
        }
    }

    #[test]
    fn a_foreign_host_port_or_scheme_is_not() {
        for url in [
            "http://127.0.0.1:9999/",
            "http://127.0.0.1:/",
            "http://127.0.0.1:17610",
            "http://localhost:17610/",
            "http://[::1]:17610/",
            "http://127.0.0.2:17610/",
            "https://127.0.0.1:17610/",
            "file:///C:/Windows/System32/calc.exe",
            "file://127.0.0.1:17610/",
            "ftp://127.0.0.1:17610/",
            "calc.exe",
            "",
            "-help",
        ] {
            assert!(!is_allowed(url, PORTS), "{url}");
        }
    }

    #[test]
    fn tricks_with_case_userinfo_backslash_and_port_are_not() {
        for url in [
            "HTTP://127.0.0.1:17610/",
            "Http://127.0.0.1:17610/",
            "http://127.0.0.1:17610@evil.example/",
            "http://127.0.0.1:17610\\@evil.example/",
            "http://127.0.0.1:17610/\\\\evil\\share",
            "http://evil.example@127.0.0.1:17610/",
            "http://127.0.0.1:017610/",
            "http://127.0.0.1:08080/",
            "http://127.0.0.1:+17610/",
            "http://127.0.0.1:17610 /",
            "http://127.0.0.1:17610/ --new-window",
            "http://127.0.0.1:17610/\"x",
            "http://127.0.0.1:17610/a\nb",
            "http://127.0.0.1:17610/a\0b",
            "http://127.0.0.1:17610/a b",
            "http://127.0.0.1:17610/ä",
            "http://127.0.0.1:99999/",
            "http://127.0.0.1:1761000/",
        ] {
            assert!(!is_allowed(url, PORTS), "{url:?}");
        }
    }

    #[test]
    fn a_link_that_is_too_long_is_not() {
        let long = format!("http://127.0.0.1:17610/{}", "a".repeat(MAX_LEN));
        assert!(!is_allowed(&long, PORTS));
    }

    #[test]
    fn an_empty_allowlist_allows_nothing() {
        assert!(!is_allowed("http://127.0.0.1:17610/", &[]));
    }

    #[test]
    fn open_refuses_before_it_starts_anything() {
        let err = open("file:///C:/Windows/notepad.exe", PORTS).unwrap_err();
        assert!(err.contains("not opened"), "{err}");
    }

    /// By hand: starts the real default browser (`cargo test -p appkit-hub -- --ignored opens_the_real`).
    #[cfg(windows)]
    #[test]
    #[ignore = "starts the default browser"]
    fn opens_the_real_default_browser() {
        open("http://127.0.0.1:17610/", PORTS).unwrap();
    }

    #[cfg(not(windows))]
    #[test]
    fn the_opener_gets_the_link_as_one_argument_and_is_absolute() {
        let url = "http://127.0.0.1:17610/#pair=ab";
        let (program, args) = opener(url);
        assert!(program.is_absolute(), "{program:?}");
        assert_eq!(args, vec![OsString::from(url)]);
    }
}
