//! Opening a file of the vault in Neovim, in a window of its own; showing a folder; asking for one.
//!
//! The app finds the file (and checks that it lies inside its vault); this module starts Neovim with
//! that one file as an argument. No shell is involved: a program and its arguments, so nothing in a
//! file name can become a command. Which window Neovim gets depends on the system: a new console on
//! Windows, the first terminal program found on Linux. Where no way is known the answer says so.
//!
//! `<PREFIX>_EDITOR_LOG` (a file) makes the opener write what it would have opened to that file
//! instead of starting anything: the seam of the tests. `<PREFIX>_PICK_RESULT` is then the folder
//! the folder dialog "chose" (empty or unset: the dialog was closed).

use std::ffi::OsString;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use appkit_core::identity::AppIdentity;

use crate::env::Env;

/// Something that can show a file in an editor.
pub trait FileOpener: Send + Sync {
    /// Start the editor on `file` (at `line` when given); does not wait for it. The error is a short
    /// text for the page: no path.
    ///
    /// # Errors
    ///
    /// Why the editor could not be started.
    fn open(&self, file: &Path, line: Option<u32>) -> Result<(), String>;

    /// Show `dir` in the file manager of the system; does not wait for it. An opener that cannot
    /// says so.
    ///
    /// # Errors
    ///
    /// Why the file manager could not be started.
    fn reveal(&self, _dir: &Path) -> Result<(), String> {
        Err("showing a folder is not supported here".to_owned())
    }

    /// Ask the person for a folder with the dialog of the system and wait for the answer. `Ok(None)`
    /// is a dialog that was closed without a choice. Blocks until the person has answered: run it off
    /// the async threads. An opener that cannot says so.
    ///
    /// # Errors
    ///
    /// Why no dialog could be shown (no program for it on this system, or one is open already).
    fn pick_folder(&self) -> Result<Option<PathBuf>, String> {
        Err("choosing a folder is not supported here".to_owned())
    }
}

/// A program and its arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub program: PathBuf,
    pub args: Vec<OsString>,
}

/// The terminal programs tried on Linux, in this order, each with what comes before the command it
/// runs.
const TERMINALS: &[(&str, &[&str])] = &[
    ("x-terminal-emulator", &["-e"]),
    ("gnome-terminal", &["--"]),
    ("konsole", &["-e"]),
    ("xfce4-terminal", &["-x"]),
    ("alacritty", &["-e"]),
    ("kitty", &[]),
    ("xterm", &["-e"]),
];

/// How to start `nvim` on `file` (at `line` when given) in a window of its own, or `None` when no
/// way is known on `os` (`std::env::consts::OS`). `find` looks a program up.
#[must_use]
pub fn plan(
    nvim: &Path,
    file: &Path,
    line: Option<u32>,
    os: &str,
    find: &dyn Fn(&str) -> Option<PathBuf>,
) -> Option<Launch> {
    // `+N` before the file puts the cursor on that line.
    let mut nvim_args: Vec<OsString> = Vec::new();
    if let Some(n) = line {
        nvim_args.push(OsString::from(format!("+{n}")));
    }
    nvim_args.push(file.as_os_str().to_owned());
    match os {
        // A console of its own is made when the process starts (see `spawn`).
        "windows" => Some(Launch {
            program: nvim.to_path_buf(),
            args: nvim_args,
        }),
        "linux" => TERMINALS.iter().find_map(|(name, before)| {
            let program = find(name)?;
            let mut args: Vec<OsString> = before.iter().map(OsString::from).collect();
            args.push(nvim.as_os_str().to_owned());
            args.extend(nvim_args.iter().cloned());
            Some(Launch { program, args })
        }),
        _ => None,
    }
}

/// The program that shows a folder in the file manager on `os`, or `None` when there is none.
/// `find` looks a program up.
///
/// Windows has none on purpose: the folder is handed to the shell (`ShellExecuteW`), which shows
/// it; the hub starts no `explorer.exe`. Elsewhere the program that opens things is started the
/// way a hub starts a program of somebody else (`launch`), as for a link.
#[must_use]
pub fn reveal_plan(dir: &Path, os: &str, find: &dyn Fn(&str) -> Option<PathBuf>) -> Option<Launch> {
    let program = match os {
        "macos" => Some(PathBuf::from("/usr/bin/open")),
        "linux" => find("xdg-open"),
        _ => None,
    };
    program.map(|program| Launch {
        program,
        args: vec![dir.as_os_str().to_owned()],
    })
}

fn args(list: &[&str]) -> Vec<OsString> {
    list.iter().map(OsString::from).collect()
}

/// The program that shows the folder dialog of the system on `os` and prints the chosen folder on
/// standard output, or `None` when there is none. `find` looks a program up.
///
/// Windows has none on purpose: its dialog is shown by the hub itself (`IFileOpenDialog`, see
/// `folder_dialog`), no program is started. macOS and Linux have no such call without a toolkit
/// of windows in the hub, so the program of the system that shows the dialog stays there
/// (`osascript`; `zenity` or `kdialog`), as a child of the hub (`appkit_core::child`).
#[must_use]
pub fn pick_plan(os: &str, find: &dyn Fn(&str) -> Option<PathBuf>) -> Option<Launch> {
    match os {
        "macos" => Some(Launch {
            program: PathBuf::from("/usr/bin/osascript"),
            args: args(&["-e", "POSIX path of (choose folder)"]),
        }),
        "linux" => find("zenity")
            .map(|program| Launch {
                program,
                args: args(&["--file-selection", "--directory"]),
            })
            .or_else(|| {
                find("kdialog").map(|program| Launch {
                    program,
                    args: args(&["--getexistingdirectory"]),
                })
            }),
        _ => None,
    }
}

/// The folder a dialog printed: its output without the line end, or `None` when the dialog was
/// closed (a failed exit, or nothing printed). Only a control character is refused here; whether the
/// folder can be a vault is for the app to say.
#[must_use]
pub fn picked_folder(stdout: &[u8], success: bool) -> Option<PathBuf> {
    if !success {
        return None;
    }
    let text = String::from_utf8_lossy(stdout);
    let text = text.trim_end_matches(['\r', '\n']);
    if text.is_empty() || text.chars().any(char::is_control) {
        return None;
    }
    Some(PathBuf::from(text))
}

/// A program of this name on `PATH` (as a file).
fn on_path(env: &Env, name: &str) -> Option<PathBuf> {
    let path = env.get("PATH")?;
    std::env::split_paths(path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

#[cfg(windows)]
fn spawn(launch: &Launch) -> std::io::Result<std::process::Child> {
    use std::os::windows::process::CommandExt as _;
    // CREATE_NEW_CONSOLE: Neovim is a console program, and the hub has none.
    std::process::Command::new(&launch.program)
        .args(&launch.args)
        .creation_flags(0x0000_0010)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
}

#[cfg(not(windows))]
fn spawn(launch: &Launch) -> std::io::Result<std::process::Child> {
    std::process::Command::new(&launch.program)
        .args(&launch.args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
}

/// Starts Neovim in a terminal window, shows folders, asks for one.
pub struct TerminalOpener {
    nvim: PathBuf,
    env: Env,
    identity: &'static AppIdentity,
    /// Whether a folder dialog is open: a second one would stand on top of the first.
    picking: AtomicBool,
}

impl TerminalOpener {
    /// An opener that starts `nvim`. `identity` names the variables of the test seam
    /// (`<PREFIX>_EDITOR_LOG`, `<PREFIX>_PICK_RESULT`).
    #[must_use]
    pub fn new(nvim: PathBuf, env: Env, identity: &'static AppIdentity) -> Self {
        Self {
            nvim,
            env,
            identity,
            picking: AtomicBool::new(false),
        }
    }

    /// Writes `line` to the test log when the seam is set: `Some` is the answer of the seam.
    fn logged(&self, line: &str) -> Option<Result<(), String>> {
        let log = self.env.path(&self.identity.var("EDITOR_LOG"))?;
        let written = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .and_then(|mut out| writeln!(out, "{line}"))
            .map_err(|_| "the test log cannot be written".to_owned());
        Some(written)
    }
}

impl FileOpener for TerminalOpener {
    fn open(&self, file: &Path, line: Option<u32>) -> Result<(), String> {
        let at = line.map_or_else(String::new, |n| format!(":{n}"));
        if let Some(seam) = self.logged(&format!("{}{at}", file.display())) {
            return seam;
        }
        let find = |name: &str| on_path(&self.env, name);
        let launch =
            plan(&self.nvim, file, line, std::env::consts::OS, &find).ok_or_else(|| {
                "no way to open a window with Neovim is known on this system".to_owned()
            })?;
        let mut child = spawn(&launch).map_err(|_| "Neovim could not be started".to_owned())?;
        // Reaped when it ends; the hub never waits for the editor.
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }

    fn reveal(&self, dir: &Path) -> Result<(), String> {
        if let Some(seam) = self.logged(&format!("reveal {}", dir.display())) {
            return seam;
        }
        self.show_folder(dir)
    }

    fn pick_folder(&self) -> Result<Option<PathBuf>, String> {
        // The seam of the tests: the folder the person "chose" (empty: the dialog was closed), the
        // call in the log.
        if let Some(seam) = self.logged("pick") {
            seam?;
            let chosen = self
                .env
                .get(&self.identity.var("PICK_RESULT"))
                .map(|c| c.to_string_lossy().into_owned());
            return Ok(chosen.and_then(|c| picked_folder(c.as_bytes(), true)));
        }
        if self.picking.swap(true, Ordering::SeqCst) {
            return Err("a folder dialog is open already".to_owned());
        }
        let answer = self.ask_for_folder();
        self.picking.store(false, Ordering::SeqCst);
        answer
    }
}

/// The folder as the shell of Windows takes it: an app hands over the folder as `canonicalize`
/// gave it (`\\?\C:\vault`), which the shell does not take everywhere; it gets the plain form
/// where that names the same folder ([`strip_verbatim`]).
#[cfg(any(windows, test))]
fn folder_for_the_shell(dir: &Path) -> PathBuf {
    strip_verbatim(dir.to_path_buf())
}

/// Start the program of a [`reveal_plan`] through `launch`; does not wait for it.
#[cfg(not(windows))]
fn start_file_manager(launch: &Launch) -> Result<(), String> {
    let failed = || "the file manager could not be started".to_owned();
    let args: Vec<&str> = launch
        .args
        .iter()
        .map(|a| a.to_str())
        .collect::<Option<_>>()
        .ok_or_else(failed)?;
    let opts = crate::launch::LaunchOpts {
        cwd: Path::new("/"),
        window: crate::launch::Window::Default,
    };
    crate::launch::spawn(&launch.program, &args, &opts)
        .map(|_pid| ())
        .map_err(|_| failed())
}

/// How long a folder dialog may stay open before it is closed: a person who walked away.
#[cfg(not(windows))]
const PICK_LIMIT: std::time::Duration = std::time::Duration::from_secs(30 * 60);

impl TerminalOpener {
    /// The shell of Windows shows the folder: no program is started by the hub.
    #[cfg(windows)]
    fn show_folder(&self, dir: &Path) -> Result<(), String> {
        let _ = self;
        crate::browser::shell_explore(&folder_for_the_shell(dir))
            .map_err(|_| "the file manager could not be started".to_owned())
    }

    /// The program of the system that opens things, started as a program of somebody else (it
    /// may become the file manager): nothing of the hub, a process group of its own.
    #[cfg(not(windows))]
    fn show_folder(&self, dir: &Path) -> Result<(), String> {
        let find = |name: &str| on_path(&self.env, name);
        let launch = reveal_plan(dir, std::env::consts::OS, &find)
            .ok_or_else(|| "no file manager is known on this system".to_owned())?;
        start_file_manager(&launch)
    }

    /// The dialog of Windows, shown by this process: no program is started.
    #[cfg(windows)]
    fn ask_for_folder(&self) -> Result<Option<PathBuf>, String> {
        let chosen = crate::folder_dialog::pick_folder()?;
        // The same rule as for a dialog that prints its answer: no control character.
        Ok(chosen.filter(|path| !path.to_string_lossy().chars().any(char::is_control)))
    }

    /// The dialog program of the system, as a child of the hub: it ends with the hub, and its
    /// start is in the log.
    #[cfg(not(windows))]
    fn ask_for_folder(&self) -> Result<Option<PathBuf>, String> {
        use appkit_core::child::{self, Environment, Leftovers, ProcError, Run, Start};

        let find = |name: &str| on_path(&self.env, name);
        let launch = pick_plan(std::env::consts::OS, &find)
            .ok_or_else(|| "no folder dialog is known on this system".to_owned())?;
        let run = Run {
            // The dialog needs the display and the session of the person: the environment is
            // the hub's.
            start: Start::new(
                format!("{} folder dialog", self.identity.binary),
                launch.program,
            )
            .args(launch.args)
            .env(Environment::Inherited {
                set: Vec::new(),
                remove: Vec::new(),
            }),
            stdin: Vec::new(),
            timeout: PICK_LIMIT,
            max_stdout: 64 * 1024,
            // A toolkit complains at length on stderr; nobody reads it.
            max_stderr: 1024 * 1024,
            leftovers: Leftovers::End,
        };
        match crate::blocking::block_on(|| child::run(run))? {
            Ok(output) => Ok(picked_folder(&output.stdout, output.code == Some(0))),
            Err(ProcError::Timeout(_)) => {
                Err("the folder dialog was open for too long and was closed".to_owned())
            }
            Err(_) => Err("the folder dialog could not be started".to_owned()),
        }
    }
}

/// The longest plain path Windows takes without long-path support (`MAX_PATH` is 260 with the
/// terminating NUL).
const PLAIN_PATH_LIMIT: usize = 259;

/// On Windows `canonicalize` answers `\\?\C:\dir\file`; Neovim and most programs want `C:\dir\file`.
/// The prefix stays where the plain form would not do: a network share (`\\?\UNC\...`), a path too
/// long to be plain, and a name Windows reads differently without it (a device name such as
/// `con.md`, a trailing dot or space). Anything else keeps its form.
#[must_use]
pub fn strip_verbatim(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix("\\\\?\\") {
        Some(rest)
            if rest.as_bytes().get(1) == Some(&b':')
                && rest.encode_utf16().count() <= PLAIN_PATH_LIMIT
                && rest.split('\\').skip(1).all(is_plain_name) =>
        {
            PathBuf::from(rest)
        }
        _ => path,
    }
}

/// A file or folder name that means the same with and without the verbatim prefix.
fn is_plain_name(name: &str) -> bool {
    if name.ends_with('.') || name.ends_with(' ') {
        return false;
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ')
        .to_ascii_uppercase();
    let device = matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|prefix| {
        // COM1 to COM9, and the superscript digits Windows reserves too (COM¹, COM², COM³).
        stem.strip_prefix(prefix).is_some_and(|n| {
            matches!(n, "¹" | "²" | "³") || (n.len() == 1 && n.as_bytes()[0].is_ascii_digit())
        })
    });
    !device
}

#[cfg(test)]
mod tests {
    use appkit_scratch::Scratch;

    use super::*;

    const ID: AppIdentity = AppIdentity {
        app: "kit-desktop",
        binary: "kit-hub",
        env_prefix: "KIT_DESKTOP",
        title: "Kit",
        default_port: 17999,
        route_ns: "kit",
    };

    fn none(_: &str) -> Option<PathBuf> {
        None
    }

    #[test]
    fn windows_starts_neovim_itself_and_linux_the_first_terminal_it_finds() {
        let nvim = PathBuf::from("/bin/nvim");
        let file = PathBuf::from("/v/a/x.md");
        let win = plan(&nvim, &file, None, "windows", &none).unwrap();
        assert_eq!(win.program, nvim);
        assert_eq!(win.args, vec![file.as_os_str().to_owned()]);

        let only_konsole =
            |name: &str| (name == "konsole").then(|| PathBuf::from("/usr/bin/konsole"));
        let l = plan(&nvim, &file, None, "linux", &only_konsole).unwrap();
        assert_eq!(l.program, PathBuf::from("/usr/bin/konsole"));
        assert_eq!(
            l.args,
            vec![
                OsString::from("-e"),
                nvim.as_os_str().to_owned(),
                file.as_os_str().to_owned()
            ]
        );
        // The first of the list wins.
        let both =
            |name: &str| matches!(name, "xterm" | "gnome-terminal").then(|| PathBuf::from(name));
        assert_eq!(
            plan(&nvim, &file, None, "linux", &both).unwrap().program,
            PathBuf::from("gnome-terminal")
        );
    }

    #[test]
    fn a_line_goes_right_before_the_file() {
        let nvim = PathBuf::from("/bin/nvim");
        let file = PathBuf::from("/v/x.md");
        let win = plan(&nvim, &file, Some(7), "windows", &none).unwrap();
        assert_eq!(
            win.args,
            vec![OsString::from("+7"), file.as_os_str().to_owned()]
        );
        let kitty = |name: &str| (name == "kitty").then(|| PathBuf::from("/usr/bin/kitty"));
        let l = plan(&nvim, &file, Some(7), "linux", &kitty).unwrap();
        assert_eq!(
            l.args,
            vec![
                nvim.as_os_str().to_owned(),
                OsString::from("+7"),
                file.as_os_str().to_owned()
            ]
        );
    }

    #[test]
    fn where_no_way_is_known_there_is_none() {
        let nvim = PathBuf::from("nvim");
        let file = PathBuf::from("x.md");
        assert!(plan(&nvim, &file, None, "linux", &none).is_none());
        assert!(plan(&nvim, &file, None, "macos", &none).is_none());
    }

    #[test]
    fn the_verbatim_prefix_of_a_windows_path_goes_and_a_share_keeps_its_form() {
        assert_eq!(
            strip_verbatim(PathBuf::from("\\\\?\\C:\\v\\a.md")),
            PathBuf::from("C:\\v\\a.md")
        );
        for same in ["\\\\?\\UNC\\server\\share\\a.md", "C:\\v\\a.md", "/v/a.md"] {
            assert_eq!(strip_verbatim(PathBuf::from(same)), PathBuf::from(same));
        }
        // Where the plain form would be read differently or not at all, the prefix stays.
        let long = format!("\\\\?\\C:\\{}\\a.md", "d".repeat(260));
        let edge = format!("\\\\?\\C:\\{}", "e".repeat(259 - 3));
        for kept in [
            long.as_str(),
            "\\\\?\\C:\\v\\con.md",
            "\\\\?\\C:\\v\\NUL",
            "\\\\?\\C:\\v\\com1.txt",
            "\\\\?\\C:\\v\\LPT9.md",
            "\\\\?\\C:\\v\\a.md.",
            "\\\\?\\C:\\v\\a.md ",
            "\\\\?\\C:\\aux\\a.md",
            "\\\\?\\C:\\v\\com¹.md",
            "\\\\?\\C:\\v\\LPT³",
        ] {
            assert_eq!(strip_verbatim(PathBuf::from(kept)), PathBuf::from(kept));
        }
        // 259 characters still go plain, and names that only look like devices do.
        assert_eq!(
            strip_verbatim(PathBuf::from(&edge)),
            PathBuf::from(&edge[4..])
        );
        // The limit counts UTF-16 units, not characters: a name of characters outside the basic
        // plane is twice as long.
        let wide_ok = format!("\\\\?\\C:\\{}", "😀".repeat(128)); // 3 + 256 = 259 units
        let wide_long = format!("\\\\?\\C:\\{}", "😀".repeat(129)); // 3 + 258 = 261 units
        assert_eq!(
            strip_verbatim(PathBuf::from(&wide_ok)),
            PathBuf::from(&wide_ok[4..])
        );
        assert_eq!(
            strip_verbatim(PathBuf::from(&wide_long)),
            PathBuf::from(&wide_long)
        );
        for plain in ["console.md", "com10.md", "auxiliary", "a.con"] {
            let verbatim = format!("\\\\?\\C:\\v\\{plain}");
            assert_eq!(
                strip_verbatim(PathBuf::from(&verbatim)),
                PathBuf::from(format!("C:\\v\\{plain}")),
                "{plain}"
            );
        }
    }

    #[test]
    fn a_folder_is_shown_by_the_file_manager_of_the_system() {
        let dir = Path::new("/v");
        let xdg = |name: &str| (name == "xdg-open").then(|| PathBuf::from("/usr/bin/xdg-open"));
        // Windows hands the folder to the shell: there is no program to start, whatever is there.
        let anything = |name: &str| Some(PathBuf::from(format!("C:/Windows/{name}")));
        assert_eq!(reveal_plan(dir, "windows", &none), None);
        assert_eq!(reveal_plan(dir, "windows", &anything), None);
        let mac = reveal_plan(dir, "macos", &none).unwrap();
        assert_eq!(mac.program, PathBuf::from("/usr/bin/open"));
        assert_eq!(mac.args, vec![dir.as_os_str().to_owned()]);
        assert_eq!(
            reveal_plan(dir, "linux", &xdg).unwrap().program,
            PathBuf::from("/usr/bin/xdg-open")
        );
        assert!(reveal_plan(dir, "linux", &none).is_none());
        assert!(reveal_plan(dir, "plan9", &xdg).is_none());
    }

    #[test]
    fn the_folder_dialog_is_the_one_of_the_system() {
        let zenity = |name: &str| (name == "zenity").then(|| PathBuf::from("/usr/bin/zenity"));
        let kdialog = |name: &str| (name == "kdialog").then(|| PathBuf::from("/usr/bin/kdialog"));
        assert_eq!(
            pick_plan("macos", &none).unwrap().program,
            PathBuf::from("/usr/bin/osascript")
        );
        assert_eq!(
            pick_plan("linux", &zenity).unwrap().program,
            PathBuf::from("/usr/bin/zenity")
        );
        assert_eq!(
            pick_plan("linux", &kdialog).unwrap().program,
            PathBuf::from("/usr/bin/kdialog")
        );
        assert!(pick_plan("linux", &none).is_none());
        assert!(pick_plan("plan9", &zenity).is_none());
    }

    /// Windows shows its dialog itself (`folder_dialog`): there is no program to start, whatever
    /// is installed. A shell with a script in its arguments is what an endpoint protection reads.
    #[test]
    fn windows_starts_no_program_for_the_folder_dialog() {
        let anything = |name: &str| Some(PathBuf::from(format!("C:/Windows/System32/{name}")));
        assert_eq!(pick_plan("windows", &none), None);
        assert_eq!(pick_plan("windows", &anything), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn without_a_dialog_program_the_answer_says_so() {
        let dir = Scratch::new("pick-none");
        let env = Env::from_pairs([("PATH", dir.to_string_lossy().into_owned())]);
        let opener = TerminalOpener::new(PathBuf::from("nvim"), env, &ID);
        assert_eq!(
            opener.pick_folder().unwrap_err(),
            "no folder dialog is known on this system"
        );
        // The refusal does not leave the dialog marked as open.
        assert_eq!(
            opener.pick_folder().unwrap_err(),
            "no folder dialog is known on this system"
        );
    }

    /// A dialog that prints a folder, played by a script named `zenity`: the answer comes back
    /// through the helper, and the child is in its log by the name of the app.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_dialog_of_linux_answers_through_the_helper() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = Scratch::new("pick-zenity");
        let zenity = dir.join("zenity");
        std::fs::write(&zenity, "#!/bin/sh\nprintf '%s\\n' '/home/me/Ordner äöü'\n").unwrap();
        std::fs::set_permissions(&zenity, std::fs::Permissions::from_mode(0o755)).unwrap();
        let env = Env::from_pairs([("PATH", dir.to_string_lossy().into_owned())]);
        let opener = TerminalOpener::new(PathBuf::from("nvim"), env, &ID);
        // A file that was just written can be busy for a moment when another test starts a
        // process at the same time.
        let mut answer = opener.pick_folder();
        for _ in 0..20 {
            if answer.is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
            answer = opener.pick_folder();
        }
        assert_eq!(answer, Ok(Some(PathBuf::from("/home/me/Ordner äöü"))));
    }

    /// The program that shows a folder is not the hub's: it is started in a process group of its
    /// own (as the opener of a link), with the folder as its one argument.
    #[cfg(not(windows))]
    #[test]
    fn the_file_manager_is_started_in_a_process_group_of_its_own() {
        let dir = Scratch::new("reveal-group");
        let out = dir.join("seen");
        let script =
            r#"{ ps -o pgid= -p $$; echo $$; echo "$1"; } > "$0.part" && mv "$0.part" "$0""#;
        let launch = Launch {
            program: PathBuf::from("/bin/sh"),
            args: vec![
                "-c".into(),
                script.into(),
                out.as_os_str().to_owned(),
                "/home/me/Ordner äöü".into(),
            ],
        };
        start_file_manager(&launch).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let seen = loop {
            if let Ok(text) = std::fs::read_to_string(&out) {
                break text;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the program wrote nothing"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        let lines: Vec<&str> = seen.lines().map(str::trim).collect();
        assert_eq!(lines.len(), 3, "{seen}");
        assert_eq!(lines[0], lines[1], "no process group of its own");
        assert_eq!(lines[2], "/home/me/Ordner äöü");

        let relative = Launch {
            program: PathBuf::from("xdg-open"),
            args: Vec::new(),
        };
        assert!(start_file_manager(&relative).is_err());
    }

    #[test]
    fn the_shell_gets_the_plain_form_of_a_folder() {
        assert_eq!(
            folder_for_the_shell(Path::new("\\\\?\\C:\\vault\\area")),
            PathBuf::from("C:\\vault\\area")
        );
        // Where the plain form would name something else, the folder stays as it was given.
        for kept in ["\\\\?\\UNC\\server\\share\\v", "\\\\?\\C:\\v\\con", "C:\\v"] {
            assert_eq!(folder_for_the_shell(Path::new(kept)), PathBuf::from(kept));
        }
    }

    /// Windows: only a folder is handed to the shell, so `reveal` cannot run a file.
    #[cfg(windows)]
    #[test]
    fn windows_shows_a_folder_only_and_starts_no_program_for_it() {
        let dir = Scratch::new("reveal-file");
        let file = dir.join("run.cmd");
        std::fs::write(&file, "exit 1").unwrap();
        let opener = TerminalOpener::new(PathBuf::from("nvim"), Env::from_pairs([("A", "b")]), &ID);
        assert_eq!(
            opener.reveal(&file).unwrap_err(),
            "the file manager could not be started"
        );
        assert!(opener.reveal(&dir.join("gone")).is_err());
    }

    #[test]
    fn a_closed_dialog_chooses_nothing_and_a_path_loses_only_its_line_end() {
        assert_eq!(
            picked_folder(b"/home/me/vault\n", true),
            Some(PathBuf::from("/home/me/vault"))
        );
        assert_eq!(
            picked_folder(b"C:/Users/me/vault\r\n", true),
            Some(PathBuf::from("C:/Users/me/vault"))
        );
        assert_eq!(picked_folder(b"/home/me/vault\n", false), None);
        assert_eq!(picked_folder(b"", true), None);
        assert_eq!(picked_folder(b"\n", true), None);
        assert_eq!(picked_folder(b"/a\x1b[0mb", true), None);
    }

    #[test]
    fn the_test_seam_writes_what_it_would_have_opened_shown_or_asked() {
        let dir = Scratch::new("editor-log");
        let log = dir.join("opened.log");
        let env = Env::from_pairs([
            ("KIT_DESKTOP_EDITOR_LOG", log.to_string_lossy().into_owned()),
            ("KIT_DESKTOP_PICK_RESULT", "/chosen".to_owned()),
        ]);
        let opener = TerminalOpener::new(PathBuf::from("nvim"), env, &ID);
        opener.open(Path::new("/v/a.md"), None).unwrap();
        opener.open(Path::new("/v/b.md"), Some(7)).unwrap();
        opener.reveal(Path::new("/v")).unwrap();
        assert_eq!(
            opener.pick_folder().unwrap(),
            Some(PathBuf::from("/chosen"))
        );
        let text = std::fs::read_to_string(&log).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 4, "{text}");
        assert!(lines[0].ends_with("a.md"), "{text}");
        assert!(lines[1].ends_with("b.md:7"), "{text}");
        assert!(lines[2].starts_with("reveal "), "{text}");
        assert_eq!(lines[3], "pick");
    }

    #[test]
    fn an_opener_that_cannot_show_or_ask_says_so() {
        struct OnlyOpens;
        impl FileOpener for OnlyOpens {
            fn open(&self, _: &Path, _: Option<u32>) -> Result<(), String> {
                Ok(())
            }
        }
        assert!(OnlyOpens.reveal(Path::new("/v")).is_err());
        assert!(OnlyOpens.pick_folder().is_err());
    }
}
