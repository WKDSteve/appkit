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

/// The one file name [`open_index_html`] opens.
const PAGE_FILE: &str = "index.html";
/// The longest relative path of the page, in bytes.
const MAX_PAGE_PATH: usize = 256;
/// The longest node name of the fragment, in bytes (before it is encoded).
const MAX_NODE: usize = 128;

/// Is `part` one folder or file name of the relative path: plain characters only, and not one
/// the system reads as something else (`.`, `..`, a name that ends in a dot, which Windows drops)?
fn plain_component(part: &str) -> bool {
    !part.is_empty()
        && part != "."
        && part != ".."
        && !part.ends_with('.')
        && part
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// Percent-encode `text` for a link: only letters, digits and `-_.~` stay as they are.
fn encode(text: &str) -> String {
    use std::fmt::Write as _;

    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// The path of a canonical file as the text of a `file:` link, or `None` for what is not a plain
/// local file (a network share, a device namespace, a name that is no text).
fn file_link(canonical: &std::path::Path) -> Option<String> {
    use std::path::Component;

    let mut link = String::from("file://");
    let mut any = false;
    for part in canonical.components() {
        match part {
            Component::Prefix(prefix) => {
                use std::path::Prefix;
                // `\\?\C:\…` is what `canonicalize` gives; a share or a device is not a local file.
                let (Prefix::Disk(letter) | Prefix::VerbatimDisk(letter)) = prefix.kind() else {
                    return None;
                };
                link.push('/');
                link.push(char::from(letter.to_ascii_uppercase()));
                link.push(':');
            }
            Component::RootDir => {}
            Component::Normal(name) => {
                link.push('/');
                link.push_str(&encode(name.to_str()?));
                any = true;
            }
            Component::CurDir | Component::ParentDir => return None,
        }
    }
    any.then_some(link)
}

/// The refusal of the page on the machine with the role `role` (the value of
/// `NVIM_MACHINE_ROLE`): the work machine does not open it.
fn refused_on(role: Option<&str>) -> Result<(), String> {
    match role {
        Some(role) if role.trim().eq_ignore_ascii_case("workstation") => {
            Err("the page of a repository is not opened on this machine: not opened".to_owned())
        }
        _ => Ok(()),
    }
}

/// The link of the page `relative` below `folder`, checked, or why it is not one.
fn page_link(folder: &std::path::Path, relative: &str) -> Result<String, String> {
    let refuse = |why: &str| Err(format!("not the page of a repository ({why}): not opened"));
    if relative.is_empty() || relative.len() > MAX_PAGE_PATH {
        return refuse("the path has no sensible length");
    }
    // Only `/` separates: a backslash, a drive, a share, a stream (`:`) or a root is not plain.
    let parts: Vec<&str> = relative.split('/').collect();
    if !parts.iter().all(|p| plain_component(p)) {
        return refuse("the path holds more than plain names below the folder");
    }
    if parts.last().copied() != Some(PAGE_FILE) {
        return refuse("the file is not index.html");
    }
    if !folder.is_absolute() {
        return refuse("the folder is not absolute");
    }
    let root = folder
        .canonicalize()
        .map_err(|_| "the folder is not there: not opened".to_owned())?;
    let target = root.join(relative);
    // The file itself is no link, and a link on the way may not lead out of the folder.
    if !std::fs::symlink_metadata(&target).is_ok_and(|m| m.is_file()) {
        return refuse("there is no such file");
    }
    let real = target
        .canonicalize()
        .map_err(|_| "the page is not there: not opened".to_owned())?;
    if !real.starts_with(&root) || real.file_name().and_then(|n| n.to_str()) != Some(PAGE_FILE) {
        return refuse("the page lies outside the folder");
    }
    file_link(&real).ok_or_else(|| "the page is not a local file: not opened".to_owned())
}

/// Is there a page `relative` below `folder` that [`open_index_html`] would open (the checks of
/// a call, without the machine)? A caller shows its button only when this holds.
#[must_use]
pub fn index_html_available(folder: &std::path::Path, relative: &str) -> bool {
    page_link(folder, relative).is_ok()
}

/// Open the page `relative` of a repository in the default browser: the one file named
/// `index.html`, below `folder`, which the caller has checked as the folder of a repository. Not
/// a way to open files: the name is fixed, the path may hold plain names only (no `..`, no
/// drive, no share, no backslash), the file and what it resolves to must lie in `folder` (a link
/// out of it is refused), and the link is built here from that file, never from text of the
/// caller. `node` names a node of the page: it becomes the fragment `#tab=tree&id=<node>`,
/// encoded, and it is at most 128 bytes; nothing else of the caller reaches the link.
///
/// Not opened on the work machine (`NVIM_MACHINE_ROLE` is `workstation`); a caller hides its
/// button there.
///
/// # Errors
///
/// The machine is the work machine, the path or the node is not allowed, the file is not a
/// regular file in `folder`, or the system could not open it. Whatever the error, nothing is
/// started.
pub fn open_index_html(
    folder: &std::path::Path,
    relative: &str,
    node: Option<&str>,
) -> Result<(), String> {
    refused_on(std::env::var("NVIM_MACHINE_ROLE").ok().as_deref())?;
    let link = page_link_with_node(folder, relative, node)?;
    hand_over(&link)
}

fn page_link_with_node(
    folder: &std::path::Path,
    relative: &str,
    node: Option<&str>,
) -> Result<String, String> {
    let mut link = page_link(folder, relative)?;
    if let Some(node) = node {
        if node.is_empty() || node.len() > MAX_NODE || node.chars().any(char::is_control) {
            return Err("the node is not one the page shows: not opened".to_owned());
        }
        link.push_str("#tab=tree&id=");
        link.push_str(&encode(node));
    }
    if link.len() > MAX_LEN {
        return Err("the link is too long: not opened".to_owned());
    }
    Ok(link)
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
    #[cfg(windows)]
    use std::path::Path;

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

    fn repo() -> appkit_scratch::Scratch {
        let dir = appkit_scratch::Scratch::new("browser-page");
        std::fs::create_dir_all(dir.join("docs/map")).unwrap();
        std::fs::write(dir.join("docs/map/index.html"), "<html>").unwrap();
        std::fs::write(dir.join("docs/map/other.html"), "<html>").unwrap();
        std::fs::write(dir.join("index.html"), "<html>").unwrap();
        dir
    }

    #[test]
    fn the_page_of_a_repository_gets_a_link_built_from_the_file() {
        let dir = repo();
        let link = page_link_with_node(&dir, "docs/map/index.html", None).unwrap();
        assert!(link.starts_with("file:///"), "{link}");
        assert!(link.ends_with("/docs/map/index.html"), "{link}");
        assert!(!link.contains('\\') && !link.contains('#'), "{link}");
        assert!(page_link_with_node(&dir, "index.html", None).is_ok());
        assert!(index_html_available(&dir, "docs/map/index.html"));
    }

    #[test]
    fn the_node_becomes_a_fixed_encoded_fragment() {
        let dir = repo();
        let link = page_link_with_node(&dir, "index.html", Some("a b&id=x#y/é")).unwrap();
        let (_, fragment) = link.split_once('#').unwrap();
        assert_eq!(fragment, "tab=tree&id=a%20b%26id%3Dx%23y%2F%C3%A9");
        assert_eq!(link.matches('#').count(), 1);
    }

    #[test]
    fn a_node_that_is_empty_long_or_a_control_is_refused() {
        let dir = repo();
        for node in ["", "a\nb", "a\0b", &"n".repeat(MAX_NODE + 1)] {
            assert!(
                page_link_with_node(&dir, "index.html", Some(node)).is_err(),
                "{node:?}"
            );
        }
        let longest = "n".repeat(MAX_NODE);
        assert!(page_link_with_node(&dir, "index.html", Some(&longest)).is_ok());
    }

    #[test]
    fn a_path_out_of_the_folder_or_with_another_name_opens_nothing() {
        let dir = repo();
        for relative in [
            "../index.html",
            "docs/../index.html",
            "docs/map/../../index.html",
            "./index.html",
            "/index.html",
            "//server/share/index.html",
            "C:/index.html",
            "C:index.html",
            "docs\\map\\index.html",
            "..\\index.html",
            "\\\\server\\share\\index.html",
            "index.html:stream",
            "index.html/",
            "docs//map/index.html",
            "docs/map/other.html",
            "docs/map/INDEX.HTML",
            "docs/map/index.htm",
            "docs/map/index.html.",
            "docs/map",
            "docs/map/",
            "docs",
            "",
            "index.html\0",
            "docs/map/index.html?x=1",
            "docs/map/index.html#x",
            "docs/m ap/index.html",
        ] {
            assert!(!index_html_available(&dir, relative), "{relative:?}");
            assert!(
                page_link_with_node(&dir, relative, None).is_err(),
                "{relative:?}"
            );
        }
        let long = format!("{}/index.html", "a/".repeat(MAX_PAGE_PATH));
        assert!(page_link(&dir, &long).is_err());
    }

    #[test]
    fn a_folder_that_is_relative_gone_or_without_the_file_opens_nothing() {
        let dir = repo();
        assert!(page_link(Path::new("docs"), "index.html").is_err());
        assert!(page_link(&dir.join("gone"), "index.html").is_err());
        assert!(page_link(&dir.join("docs"), "index.html").is_err());
        // A folder named index.html is not a file.
        std::fs::create_dir_all(dir.join("x/index.html")).unwrap();
        assert!(page_link(&dir, "x/index.html").is_err());
    }

    /// A link out of the folder: the file itself, or a folder on the way. Where the system may not
    /// create links, the case is not applicable.
    #[test]
    fn a_link_that_leads_out_of_the_folder_opens_nothing() {
        let outside = appkit_scratch::Scratch::new("browser-outside");
        std::fs::write(outside.join("index.html"), "<html>").unwrap();
        let dir = repo();
        #[cfg(windows)]
        let (file_link, dir_link) = (
            std::os::windows::fs::symlink_file(outside.join("index.html"), dir.join("link.html")),
            std::os::windows::fs::symlink_dir(outside.path(), dir.join("out")),
        );
        #[cfg(not(windows))]
        let (file_link, dir_link) = (
            std::os::unix::fs::symlink(outside.join("index.html"), dir.join("link.html")),
            std::os::unix::fs::symlink(outside.path(), dir.join("out")),
        );
        if file_link.is_err() || dir_link.is_err() {
            println!("n/a  this system does not let the test create links");
            return;
        }
        // A file that is a link is refused, even to a file that is named right.
        std::fs::create_dir_all(dir.join("viafile")).unwrap();
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_file(
            outside.join("index.html"),
            dir.join("viafile/index.html"),
        );
        #[cfg(not(windows))]
        let made =
            std::os::unix::fs::symlink(outside.join("index.html"), dir.join("viafile/index.html"));
        assert!(made.is_ok());
        assert!(page_link(&dir, "viafile/index.html").is_err());
        // A folder on the way that leads out.
        assert!(page_link(&dir, "out/index.html").is_err());
    }

    #[test]
    fn the_work_machine_does_not_open_the_page() {
        for role in ["workstation", "Workstation", " WORKSTATION "] {
            let err = refused_on(Some(role)).unwrap_err();
            assert!(err.contains("not opened"), "{err}");
        }
        for role in [None, Some(""), Some("default"), Some("workstations")] {
            assert!(refused_on(role).is_ok(), "{role:?}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn only_a_local_disk_is_a_link() {
        for share in [
            r"\\server\share\docs\index.html",
            r"\\?\UNC\server\share\index.html",
        ] {
            assert_eq!(file_link(Path::new(share)), None, "{share}");
        }
        assert_eq!(
            file_link(Path::new(r"\\?\C:\my repo\index.html")).as_deref(),
            Some("file:///C:/my%20repo/index.html")
        );
        assert_eq!(
            file_link(Path::new(r"d:\r\index.html")).as_deref(),
            Some("file:///D:/r/index.html")
        );
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
