//! Open a page of a local hub in the default browser.
//!
//! Only `http://127.0.0.1:<port>/…` with a port the app names. Never a name other than the loopback
//! address, and never a file a caller names: the one file that is opened is the page of a pairing
//! code the hub wrote itself ([`open_own_page`], `phone::show_qr`), by its fixed name. On Windows the link
//! goes to `ShellExecuteW` (verb `open`), no helper program is started; elsewhere the program that
//! opens links gets exactly one argument, the link, and it is an absolute program. That program is
//! not a child of the app (it may become the browser): it is started the way a hub starts a program
//! of somebody else ([`crate::launch`]), with nothing of the hub and in a process group of its own.

#[cfg(not(windows))]
use std::path::{Path, PathBuf};

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
fn opener(url: &str) -> (PathBuf, Vec<String>) {
    if cfg!(target_os = "macos") {
        (PathBuf::from("/usr/bin/open"), vec![url.to_owned()])
    } else {
        (PathBuf::from("/usr/bin/xdg-open"), vec![url.to_owned()])
    }
}

/// Start the program that opens links. It is not the app's: `launch::spawn` gives it the null
/// device as its streams and a process group of its own, and does not wait for it.
#[cfg(not(windows))]
fn start_opener(program: &Path, args: &[String]) -> Result<(), String> {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let opts = crate::launch::LaunchOpts {
        cwd: Path::new("/"),
        window: crate::launch::Window::Default,
    };
    crate::launch::spawn(program, &args, &opts)
        .map(|_pid| ())
        .map_err(|e| format!("could not start the browser: {e}"))
}

/// Hand `url` to the shell (`ShellExecuteW`, verb `open`): no helper program is started, so no
/// `rundll32` (a known living-off-the-land binary that endpoint protection watches for).
#[cfg(windows)]
fn shell_open(url: &str) -> Result<(), String> {
    shell("open", std::ffi::OsStr::new(url))
        .map_err(|code| format!("could not start the browser: the shell answered {code}"))
}

/// Show the folder `dir` in the file manager (`ShellExecuteW`, verb `explore`): no
/// `explorer.exe` is started by the hub. The verb exists for folders only, and `dir` must be one:
/// a file is never handed over, so nothing can be run this way.
///
/// # Errors
///
/// `dir` is not a folder, or the shell refused.
#[cfg(windows)]
pub(crate) fn shell_explore(dir: &std::path::Path) -> Result<(), String> {
    if !dir.is_absolute() || !dir.is_dir() {
        return Err("not a folder: not shown".to_owned());
    }
    crate::handles::keep_standard_handles();
    shell("explore", dir.as_os_str())
        .map_err(|code| format!("the file manager could not be started: the shell answered {code}"))
}

/// Ask the shell to do `verb` with `target`. The error is the code of the shell.
#[cfg(windows)]
#[allow(unsafe_code)]
fn shell(verb: &str, target: &std::ffi::OsStr) -> Result<(), usize> {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let verb: Vec<u16> = verb.encode_utf16().chain(std::iter::once(0)).collect();
    let target: Vec<u16> = target.encode_wide().chain(std::iter::once(0)).collect();
    // A NUL inside would cut the text short: what is handed over is what was checked.
    if verb[..verb.len() - 1].contains(&0) || target[..target.len() - 1].contains(&0) {
        return Err(0);
    }
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
        Err(result as usize)
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
    hand_over(url)
}

/// Is `path` the page of a pairing code as the hub writes it: absolute, the fixed name
/// ([`crate::phone::QR_FILE`]), and a file that is there (no link to follow)?
#[must_use]
pub(crate) fn is_own_page(path: &std::path::Path) -> bool {
    path.is_absolute()
        && path.file_name().and_then(|n| n.to_str()) == Some(crate::phone::QR_FILE)
        && std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file())
}

/// Open the page of a pairing code the hub wrote itself, with the program the system opens a page
/// with. The one file this module opens: its name is fixed and its content is the hub's own, so no
/// caller chooses what is started.
///
/// # Errors
///
/// `path` is not that page, or the system could not open it.
pub(crate) fn open_own_page(path: &std::path::Path) -> Result<(), String> {
    if !is_own_page(path) {
        return Err("not the page of a pairing code: not opened".to_owned());
    }
    let text = path
        .to_str()
        .ok_or("the settings folder has a name that is no text: not opened")?;
    hand_over(text)
}

/// Hand a checked link or page to the system.
fn hand_over(target: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        crate::handles::keep_standard_handles();
        shell_open(target)
    }
    #[cfg(not(windows))]
    {
        let (program, args) = opener(target);
        start_opener(&program, &args)
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

    #[cfg(windows)]
    #[test]
    fn only_a_folder_is_shown_in_the_file_manager() {
        let dir = appkit_scratch::Scratch::new("browser-explore");
        let file = dir.join("run.cmd");
        std::fs::write(&file, "exit 1").unwrap();
        for not_a_folder in [file.as_path(), &dir.join("gone"), std::path::Path::new(".")] {
            let err = shell_explore(not_a_folder).unwrap_err();
            assert_eq!(err, "not a folder: not shown", "{not_a_folder:?}");
        }
        assert_eq!(shell("open", std::ffi::OsStr::new("a\0b")), Err(0));
    }

    /// By hand: opens a window of the file manager (`cargo test -p appkit-hub -- --ignored shows_a_real`).
    #[cfg(windows)]
    #[test]
    #[ignore = "opens a window of the file manager"]
    fn shows_a_real_folder_in_the_file_manager() {
        shell_explore(&std::env::current_dir().unwrap()).unwrap();
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
        assert_eq!(args, vec![url.to_owned()]);
    }

    /// The opener is a program of somebody else (it may become the browser): it leads a process
    /// group of its own, so what ends the group of the hub (a Ctrl-C in the terminal) does not
    /// reach it.
    #[cfg(not(windows))]
    #[test]
    fn the_opener_is_started_in_a_process_group_of_its_own() {
        let dir = appkit_scratch::Scratch::new("browser-opener");
        let out = dir.join("seen");
        // `$0` is the file, `$1` what the opener was given. Written: the group, the process, the link.
        let script =
            r#"{ ps -o pgid= -p $$; echo $$; echo "$1"; } > "$0.part" && mv "$0.part" "$0""#;
        let url = "http://127.0.0.1:17610/#pair=ab";
        let args = [
            "-c".to_owned(),
            script.to_owned(),
            out.to_str().unwrap().to_owned(),
            url.to_owned(),
        ];
        start_opener(Path::new("/bin/sh"), &args).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let seen = loop {
            if let Ok(text) = std::fs::read_to_string(&out) {
                break text;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the opener wrote nothing"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        let lines: Vec<&str> = seen.lines().map(str::trim).collect();
        assert_eq!(lines.len(), 3, "{seen}");
        assert_eq!(lines[0], lines[1], "the opener leads no group of its own");
        assert_eq!(lines[2], url);
    }

    #[cfg(not(windows))]
    #[test]
    fn an_opener_that_is_not_there_is_an_error() {
        let err = start_opener(Path::new("/nowhere/open"), &[]).unwrap_err();
        assert!(err.starts_with("could not start the browser"), "{err}");
    }
}
