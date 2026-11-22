//! The command of an app in the terminal: `tasks-desktop status` instead of the whole path of `tasks-hub.exe`.
//!
//! `install` writes a small starter named after the app (`tasks-desktop`, not the program `tasks-hub`) into one folder
//! that all apps of the family share, and puts that folder on the `PATH` of the user once. `uninstall` takes the
//! starter away again, and the folder and its `PATH` entry with the last one.
//!
//! - Windows: the folder is `bin` in the family folder (`%LOCALAPPDATA%\appkit-family\bin`), on the `PATH` of the user
//!   (`HKCU\Environment`, no administrator rights). Each app gets `<app>.cmd` for PowerShell and cmd, and `<app>` (a
//!   shell script) for Git Bash. A starter and not a copy of the program: the hubs are window programs, and PowerShell
//!   does not wait for a window program, so its output would come after the next prompt and `$LASTEXITCODE` would be
//!   wrong; cmd waits for the program a `.cmd` starts and hands its exit code back. A copy would also be old after the
//!   next install.
//! - Linux and macOS: the shell script in `~/.local/bin`, a folder the system already has on `PATH` (it is not added
//!   to the start files of any shell; `install` says when it is missing).
//!
//! A starter only starts the program with all arguments and hands its exit code back (3 when the program is gone). It
//! carries [`MARK`], so a file of the same name that is not a starter is never overwritten or removed.

use std::path::{Path, PathBuf};

use appkit_core::family;

use crate::env::Env;

/// The line every starter carries; a file without it is not one of ours.
pub const MARK: &str = "appkit starter";

/// The exit code of a starter whose program is gone.
pub const PROGRAM_GONE: i32 = 3;

/// The value of the `PATH` of the user, as far as the starters need it: read with its type, written with one.
pub trait UserPath {
    /// The value as it is stored (`%VAR%` not expanded) and whether it is an expandable string; `None` when there is
    /// none.
    ///
    /// # Errors
    ///
    /// It could not be read.
    fn read(&self) -> Result<Option<(String, bool)>, String>;

    /// Store `value`, expandable when `expand` is set.
    ///
    /// # Errors
    ///
    /// It could not be written.
    fn write(&self, value: &str, expand: bool) -> Result<(), String>;

    /// Tell the running programs that it changed (Windows: `WM_SETTINGCHANGE`).
    fn announce(&self) {}
}

/// The `PATH` of the user in the registry of the system (Windows only).
pub struct SystemPath;

#[cfg(windows)]
const ENVIRONMENT_KEY: &str = r"HKCU\Environment";

impl UserPath for SystemPath {
    #[cfg(windows)]
    fn read(&self) -> Result<Option<(String, bool)>, String> {
        crate::winreg::get_typed(ENVIRONMENT_KEY, "Path")
    }

    #[cfg(not(windows))]
    fn read(&self) -> Result<Option<(String, bool)>, String> {
        Err("this system keeps no PATH of the user to change".to_owned())
    }

    #[cfg(windows)]
    fn write(&self, value: &str, expand: bool) -> Result<(), String> {
        crate::winreg::set_typed(ENVIRONMENT_KEY, "Path", value, expand)
    }

    #[cfg(not(windows))]
    fn write(&self, _value: &str, _expand: bool) -> Result<(), String> {
        Err("this system keeps no PATH of the user to change".to_owned())
    }

    #[cfg(windows)]
    fn announce(&self) {
        crate::winreg::announce_environment();
    }
}

/// Whether the starter folder is the family's own (`bin` in the family folder), which the starters may put on `PATH`
/// and remove when it is empty; `~/.local/bin` is the person's.
fn owns_dir(env: &Env) -> bool {
    cfg!(windows) || env.get(family::FAMILY_DIR_VAR).is_some()
}

/// The folder of the starters: `bin` in the family folder on Windows (and wherever `APPKIT_FAMILY_DIR` points, which is
/// what the tests do), `~/.local/bin` elsewhere. `None` without `LOCALAPPDATA` or `HOME`.
#[must_use]
pub fn starter_dir(env: &Env) -> Option<PathBuf> {
    if owns_dir(env) {
        return family::family_dir(&|name: &str| env.path(name)).map(|d| d.join("bin"));
    }
    env.path("HOME").map(|h| h.join(".local").join("bin"))
}

/// The starter files of `command` in `dir`: on Windows `<command>.cmd` and `<command>` (Git Bash), elsewhere only the
/// shell script.
#[must_use]
pub fn starter_files(dir: &Path, command: &str) -> Vec<PathBuf> {
    if cfg!(windows) {
        vec![dir.join(format!("{command}.cmd")), dir.join(command)]
    } else {
        vec![dir.join(command)]
    }
}

/// A command name the starters accept: the name of an app (`tasks-desktop`), nothing a shell or a path reads.
fn plain_command(command: &str) -> bool {
    !command.is_empty()
        && command.len() <= 64
        && command
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
        && !command.starts_with(['-', '.'])
}

/// `path` under `base` (Windows: without case), the rest; `None` when it is not.
fn below(path: &str, base: &str) -> Option<String> {
    let base = base.trim_end_matches(['\\', '/']);
    if base.is_empty() || path.len() <= base.len() || !path.is_char_boundary(base.len()) {
        return None;
    }
    let (head, rest) = path.split_at(base.len());
    let same = if cfg!(windows) {
        head.eq_ignore_ascii_case(base)
    } else {
        head == base
    };
    (same && rest.starts_with(['\\', '/'])).then(|| rest.to_owned())
}

/// The program as the `.cmd` names it: below `LOCALAPPDATA` as `%LOCALAPPDATA%\...` (so a name with any character in
/// the user folder reaches the program as it is), else the whole path with `%` doubled. cmd reads a `.cmd` in the code
/// page of the console, so the rest must be plain ASCII without a quote.
fn cmd_program(program: &Path, env: &Env) -> Result<String, String> {
    let whole = program.to_string_lossy().replace('/', "\\");
    let named = match env
        .get("LOCALAPPDATA")
        .and_then(|l| below(&whole, &l.to_string_lossy()))
    {
        Some(rest) => format!("%LOCALAPPDATA%{}", rest.replace('%', "%%")),
        None => whole.replace('%', "%%"),
    };
    if named
        .chars()
        .any(|c| !c.is_ascii() || c.is_ascii_control() || c == '"')
    {
        return Err(format!(
            "cmd cannot read the path {} in a starter (a character that is not plain ASCII, or a quote)",
            program.display()
        ));
    }
    Ok(named)
}

/// The `.cmd` starter of `command` for `program`.
///
/// # Errors
///
/// The program cannot be named in a `.cmd` (see [`cmd_program`]).
pub fn cmd_text(command: &str, program: &Path, env: &Env) -> Result<String, String> {
    let named = cmd_program(program, env)?;
    // One-line ifs: a path with ")" (Program Files (x86)) would end a block in parentheses early.
    Ok(format!(
        "@echo off\r\n\
         rem {MARK} of {command}: starts its program with all arguments and hands its exit code back.\r\n\
         rem Written by the install of {command}; it goes with its uninstall.\r\n\
         setlocal\r\n\
         set \"program={named}\"\r\n\
         if not exist \"%program%\" echo {command}: its program is gone (%program%): install it again 1>&2& exit /b {PROGRAM_GONE}\r\n\
         \"%program%\" %*\r\n\
         exit /b %ERRORLEVEL%\r\n"
    ))
}

/// The shell starter of `command` for `program` (Git Bash on Windows, any shell elsewhere).
///
/// # Errors
///
/// The path holds a control character.
pub fn sh_text(command: &str, program: &Path) -> Result<String, String> {
    let mut path = program.to_string_lossy().into_owned();
    if path.chars().any(char::is_control) {
        return Err(format!(
            "a shell cannot be given the path {} in a starter",
            program.display()
        ));
    }
    if cfg!(windows) {
        // Git Bash takes `C:/...` as it is.
        path = path.replace('\\', "/");
    }
    let quoted = format!("'{}'", path.replace('\'', r"'\''"));
    Ok(format!(
        "#!/bin/sh\n\
         # {MARK} of {command}: starts its program with all arguments and hands its exit code back.\n\
         # Written by the install of {command}; it goes with its uninstall.\n\
         program={quoted}\n\
         if [ ! -f \"$program\" ]; then\n  \
           echo \"{command}: its program is gone ($program): install it again\" >&2\n  \
           exit {PROGRAM_GONE}\n\
         fi\n\
         exec \"$program\" \"$@\"\n"
    ))
}

/// Whether the file at `path` is one of the starters (it carries [`MARK`]); `false` when it is not there.
fn is_starter(path: &Path) -> bool {
    std::fs::read(path).is_ok_and(|bytes| bytes.windows(MARK.len()).any(|w| w == MARK.as_bytes()))
}

/// Write `text` to `path` (beside, then moved over), executable elsewhere than on Windows. A file there that is not
/// a starter is left as it is.
fn write_starter(path: &Path, text: &str) -> Result<(), String> {
    if path.exists() && !is_starter(path) {
        return Err(format!(
            "{} is there and is not a starter: it is left as it is",
            path.display()
        ));
    }
    let part = path.with_extension("appkit-new");
    std::fs::write(&part, text).map_err(|e| format!("cannot write {}: {e}", part.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(&part, std::fs::Permissions::from_mode(0o755)) {
            let _ = std::fs::remove_file(&part);
            return Err(format!("cannot make {} executable: {e}", part.display()));
        }
    }
    std::fs::rename(&part, path).map_err(|e| {
        let _ = std::fs::remove_file(&part);
        format!("cannot write {}: {e}", path.display())
    })
}

/// `%NAME%` in `entry` replaced by the variable of `env` (one that is not set stays as it is).
fn expand(entry: &str, env: &Env) -> String {
    let mut out = String::new();
    let mut rest = entry;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 => {
                let name = &after[..end];
                match env.get(name) {
                    Some(value) => out.push_str(&value.to_string_lossy()),
                    None => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Whether the `PATH` entry `entry` names `dir`: Windows compares without case, without a trailing separator and with
/// `%VAR%` expanded.
fn names_dir(entry: &str, dir: &Path, env: &Env) -> bool {
    let entry = expand(entry.trim(), env);
    let dir = dir.to_string_lossy();
    let (a, b) = (
        entry.trim_end_matches(['\\', '/']),
        dir.trim_end_matches(['\\', '/']),
    );
    if cfg!(windows) {
        a.replace('/', "\\")
            .eq_ignore_ascii_case(&b.replace('/', "\\"))
    } else {
        a == b
    }
}

/// The separator of `PATH`.
const SEP: char = if cfg!(windows) { ';' } else { ':' };

/// Whether the `PATH` value `value` holds `dir`.
fn path_holds(value: &str, dir: &Path, env: &Env) -> bool {
    value.split(SEP).any(|e| names_dir(e, dir, env))
}

/// `value` with `entry` at its end (an empty value is the entry alone; no empty entry is made).
fn path_with(value: &str, entry: &str) -> String {
    let kept = value.trim_end_matches(SEP);
    if kept.trim().is_empty() {
        entry.to_owned()
    } else {
        format!("{kept}{SEP}{entry}")
    }
}

/// `value` without the entries that name `dir`; `None` when there was none.
fn path_without(value: &str, dir: &Path, env: &Env) -> Option<String> {
    let entries: Vec<&str> = value.split(SEP).collect();
    let kept: Vec<&str> = entries
        .iter()
        .copied()
        .filter(|e| !names_dir(e, dir, env))
        .collect();
    (kept.len() != entries.len()).then(|| kept.join(&SEP.to_string()))
}

/// How the starter folder is written into the `PATH` of the user: in an expandable value as
/// `%LOCALAPPDATA%\appkit-family\bin` (it follows a moved user folder), else as the whole path.
fn path_entry(dir: &Path, env: &Env, expandable: bool) -> String {
    let whole = dir.to_string_lossy().into_owned();
    if expandable
        && let Some(rest) = env
            .get("LOCALAPPDATA")
            .and_then(|l| below(&whole, &l.to_string_lossy()))
    {
        return format!("%LOCALAPPDATA%{rest}");
    }
    whole
}

/// Write the starter of `command` for `program` and put the starter folder on `PATH` through `path` (`None`: the
/// `PATH` of the user is not changed, only the one of this process is looked at). Returns the lines for the person.
///
/// # Errors
///
/// The command name is not plain, there is no starter folder, or a starter cannot be written. A `PATH` that cannot be
/// changed is no error: the lines say it, the starter works when called with its folder.
pub fn install(
    command: &str,
    program: &Path,
    env: &Env,
    path: Option<&dyn UserPath>,
) -> Result<String, String> {
    if !plain_command(command) {
        return Err(format!("not a name for a command: {command:?}"));
    }
    let dir = starter_dir(env).ok_or("no folder for the command (no LOCALAPPDATA or HOME)")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    for file in starter_files(&dir, command) {
        let text = if file.extension().is_some_and(|x| x == "cmd") {
            cmd_text(command, program, env)?
        } else {
            sh_text(command, program)?
        };
        write_starter(&file, &text)?;
    }
    let mut out = format!(
        "command: `{command}` in any new terminal (starter in {})\n",
        dir.display()
    );
    out.push_str(&put_on_path(&dir, env, path));
    Ok(out)
}

/// The step of [`install`] that puts `dir` on the `PATH` of the user; the line for the person.
fn put_on_path(dir: &Path, env: &Env, path: Option<&dyn UserPath>) -> String {
    let Some(store) = path else {
        let on = env
            .get("PATH")
            .is_some_and(|v| path_holds(&v.to_string_lossy(), dir, env));
        return if on {
            String::new()
        } else {
            format!(
                "{} is not on PATH: add it there to call the command by name\n",
                dir.display()
            )
        };
    };
    let now = match store.read() {
        Ok(now) => now,
        Err(e) => {
            return format!(
                "the PATH of the user cannot be read ({e}): add {} to it\n",
                dir.display()
            )
        }
    };
    if now
        .as_ref()
        .is_some_and(|(value, _)| path_holds(value, dir, env))
    {
        return String::new();
    }
    // A new value is expandable, as Windows makes it; an existing one keeps its type.
    let expandable = now.as_ref().is_none_or(|(_, x)| *x);
    let entry = path_entry(dir, env, expandable);
    let value = path_with(now.as_ref().map_or("", |(v, _)| v.as_str()), &entry);
    match store.write(&value, expandable) {
        Ok(()) => {
            store.announce();
            format!(
                "{} is now on the PATH of your user: open a new terminal (in this PowerShell: $env:Path += ';{}')\n",
                dir.display(),
                dir.display()
            )
        }
        Err(e) => format!(
            "the PATH of the user cannot be written ({e}): add {} to it\n",
            dir.display()
        ),
    }
}

/// Remove the starter of `command`, and with the last starter the folder and its `PATH` entry (through `path`; only
/// when the folder is the family's). Returns the lines for the person.
///
/// # Errors
///
/// A starter that is there cannot be removed.
pub fn uninstall(command: &str, env: &Env, path: Option<&dyn UserPath>) -> Result<String, String> {
    if !plain_command(command) {
        return Err(format!("not a name for a command: {command:?}"));
    }
    let Some(dir) = starter_dir(env) else {
        return Ok(String::new());
    };
    let mut out = String::new();
    let mut removed = false;
    for file in starter_files(&dir, command) {
        if !is_starter(&file) {
            continue;
        }
        std::fs::remove_file(&file)
            .map_err(|e| format!("cannot remove {}: {e}", file.display()))?;
        removed = true;
    }
    if removed {
        out.push_str(&format!("command `{command}` removed\n"));
    }
    if !owns_dir(env) {
        return Ok(out);
    }
    let empty = std::fs::read_dir(&dir).is_ok_and(|mut d| d.next().is_none());
    if !empty {
        return Ok(out);
    }
    let _ = std::fs::remove_dir(&dir);
    if let Some(store) = path
        && let Ok(Some((value, expandable))) = store.read()
        && let Some(kept) = path_without(&value, &dir, env)
    {
        match store.write(&kept, expandable) {
            Ok(()) => {
                store.announce();
                out.push_str(&format!(
                    "{} is off the PATH of your user again\n",
                    dir.display()
                ));
            }
            Err(e) => out.push_str(&format!(
                "{} stays on the PATH of your user: {e}\n",
                dir.display()
            )),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use appkit_scratch::Scratch;
    use std::cell::RefCell;

    /// A `PATH` of the user in memory: the tests never change the one of the machine.
    #[derive(Default)]
    struct FakePath {
        value: RefCell<Option<(String, bool)>>,
        announced: RefCell<u32>,
    }

    impl UserPath for FakePath {
        fn read(&self) -> Result<Option<(String, bool)>, String> {
            Ok(self.value.borrow().clone())
        }
        fn write(&self, value: &str, expand: bool) -> Result<(), String> {
            *self.value.borrow_mut() = Some((value.to_owned(), expand));
            Ok(())
        }
        fn announce(&self) {
            *self.announced.borrow_mut() += 1;
        }
    }

    fn scratch_env(root: &Path) -> Env {
        Env::from_pairs([
            (family::FAMILY_DIR_VAR, root.join("family")),
            ("LOCALAPPDATA", root.join("local")),
            ("HOME", root.join("home")),
        ])
    }

    #[test]
    fn only_the_name_of_an_app_is_a_command() {
        for good in ["tasks-desktop", "hub-desktop", "ai-desktop", "docmap"] {
            assert!(plain_command(good), "{good}");
        }
        for bad in [
            "", "-x", ".x", "a b", "a/b", r"a\b", "A", "x&y", "x%y", "x\"y",
        ] {
            assert!(!plain_command(bad), "{bad:?}");
        }
    }

    #[test]
    fn the_cmd_starter_names_a_program_below_the_user_folder_by_its_variable() {
        let env = Env::from_pairs([("LOCALAPPDATA", r"C:\Users\Jürgen\AppData\Local")]);
        let program = Path::new(r"C:\Users\Jürgen\AppData\Local\tasks-desktop\bin\tasks-hub.exe");
        if cfg!(windows) {
            let text = cmd_text("tasks-desktop", program, &env).unwrap();
            assert!(
                text.contains(
                    "set \"program=%LOCALAPPDATA%\\tasks-desktop\\bin\\tasks-hub.exe\"\r\n"
                ),
                "{text}"
            );
            assert!(text.contains(MARK), "{text}");
            assert!(text.contains("\"%program%\" %*\r\n"), "{text}");
            assert!(text.contains("exit /b %ERRORLEVEL%"), "{text}");
            assert!(text.contains(&format!("exit /b {PROGRAM_GONE}")), "{text}");
            // Elsewhere the umlaut would reach cmd in the code page of the console: refused, not mangled.
            let other = Path::new(r"D:\Jürgen\tasks-hub.exe");
            assert!(cmd_text("tasks-desktop", other, &env).is_err());
            // A percent sign of a path is not a variable.
            let percent = Path::new(r"D:\100%\tasks-hub.exe");
            assert!(cmd_text("tasks-desktop", percent, &env)
                .unwrap()
                .contains(r"D:\100%%\tasks-hub.exe"));
        }
    }

    #[test]
    fn the_shell_starter_quotes_the_path_and_hands_everything_on() {
        let text = sh_text("tasks-desktop", Path::new("/opt/it's/tasks-hub")).unwrap();
        assert!(text.starts_with("#!/bin/sh\n"), "{text}");
        assert!(text.contains(MARK), "{text}");
        assert!(text.contains(r"program='/opt/it'\''s/tasks-hub'"), "{text}");
        assert!(text.contains("exec \"$program\" \"$@\"\n"), "{text}");
        if cfg!(windows) {
            let win = sh_text("x", Path::new(r"C:\A B\x.exe")).unwrap();
            assert!(win.contains("program='C:/A B/x.exe'"), "{win}");
        }
        assert!(sh_text("x", Path::new("/a\nb")).is_err());
    }

    #[test]
    fn path_entries_are_compared_as_windows_does() {
        let env = Env::from_pairs([("LOCALAPPDATA", if cfg!(windows) { r"C:\L" } else { "/l" })]);
        let dir = if cfg!(windows) {
            PathBuf::from(r"C:\L\appkit-family\bin")
        } else {
            PathBuf::from("/l/appkit-family/bin")
        };
        let entry = path_entry(&dir, &env, true);
        if cfg!(windows) {
            assert_eq!(entry, r"%LOCALAPPDATA%\appkit-family\bin");
            assert_eq!(path_entry(&dir, &env, false), r"C:\L\appkit-family\bin");
            assert!(path_holds(r"C:\x;c:\l\APPKIT-FAMILY\bin\", &dir, &env));
            assert!(path_holds(r"%LOCALAPPDATA%\appkit-family\bin", &dir, &env));
            assert!(!path_holds(r"C:\x;C:\L\appkit-family", &dir, &env));
            assert_eq!(path_with("", "e"), "e");
            assert_eq!(path_with(r"C:\a;", "e"), r"C:\a;e");
            assert_eq!(path_with(r"C:\a", "e"), r"C:\a;e");
            assert_eq!(
                path_without(
                    r"C:\a;%LOCALAPPDATA%\appkit-family\bin;C:\b;c:\l\appkit-family\bin",
                    &dir,
                    &env
                ),
                Some(r"C:\a;C:\b".to_owned())
            );
            assert_eq!(path_without(r"C:\a;C:\b", &dir, &env), None);
        } else {
            assert!(path_holds("/x:/l/appkit-family/bin/", &dir, &env));
            assert_eq!(path_with("/a:", "e"), "/a:e");
        }
        // An unknown variable and a lone percent sign stay as they are.
        assert_eq!(expand("%NOPE%\\x%", &env), "%NOPE%\\x%");
    }

    #[test]
    fn install_writes_the_starters_and_puts_the_folder_on_the_path_once() {
        let root = Scratch::new("cli-entry-install");
        let env = scratch_env(&root);
        let program = root
            .join("local")
            .join("tasks-desktop")
            .join("bin")
            .join("tasks-hub.exe");
        let store = FakePath::default();
        *store.value.borrow_mut() = Some((r"%USERPROFILE%\x".to_owned(), true));
        let said = install("tasks-desktop", &program, &env, Some(&store)).unwrap();
        let dir = starter_dir(&env).unwrap();
        assert_eq!(dir, root.join("family").join("bin"));
        for file in starter_files(&dir, "tasks-desktop") {
            assert!(is_starter(&file), "{}", file.display());
        }
        assert!(said.contains("command: `tasks-desktop`"), "{said}");
        assert!(said.contains("is now on the PATH of your user"), "{said}");
        let (value, expandable) = store.value.borrow().clone().unwrap();
        // The type of an expandable value stays, and so do its other entries.
        assert!(expandable);
        assert!(value.starts_with(r"%USERPROFILE%\x"), "{value}");
        assert!(path_holds(&value, &dir, &env), "{value}");
        assert_eq!(*store.announced.borrow(), 1);

        // A second install writes the starters again and adds no second entry.
        let again = install("tasks-desktop", &program, &env, Some(&store)).unwrap();
        assert!(!again.contains("now on the PATH"), "{again}");
        assert_eq!(store.value.borrow().clone().unwrap().0, value);
        assert_eq!(*store.announced.borrow(), 1);
    }

    #[test]
    fn a_plain_path_value_stays_plain_and_gets_the_whole_folder() {
        let root = Scratch::new("cli-entry-plain");
        let env = scratch_env(&root);
        let store = FakePath::default();
        *store.value.borrow_mut() = Some(("/a".to_owned(), false));
        install("rules-desktop", &root.join("r.exe"), &env, Some(&store)).unwrap();
        let (value, expandable) = store.value.borrow().clone().unwrap();
        assert!(!expandable);
        assert!(!value.contains('%'), "{value}");
        assert!(
            path_holds(&value, &starter_dir(&env).unwrap(), &env),
            "{value}"
        );
    }

    #[test]
    fn a_file_of_the_same_name_that_is_not_a_starter_is_left_alone() {
        let root = Scratch::new("cli-entry-foreign");
        let env = scratch_env(&root);
        let dir = starter_dir(&env).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        let mine = starter_files(&dir, "tasks-desktop")[0].clone();
        std::fs::write(&mine, "the person's own file").unwrap();
        let err = install("tasks-desktop", &root.join("t.exe"), &env, None).unwrap_err();
        assert!(err.contains("is not a starter"), "{err}");
        assert_eq!(
            std::fs::read_to_string(&mine).unwrap(),
            "the person's own file"
        );
        uninstall("tasks-desktop", &env, None).unwrap();
        assert!(mine.exists());
    }

    #[test]
    fn uninstall_takes_the_folder_off_the_path_with_the_last_starter_only() {
        let root = Scratch::new("cli-entry-uninstall");
        let env = scratch_env(&root);
        let store = FakePath::default();
        *store.value.borrow_mut() = Some((r"C:\keep".to_owned(), true));
        install("tasks-desktop", &root.join("t.exe"), &env, Some(&store)).unwrap();
        install("rules-desktop", &root.join("r.exe"), &env, Some(&store)).unwrap();
        let dir = starter_dir(&env).unwrap();

        let first = uninstall("tasks-desktop", &env, Some(&store)).unwrap();
        assert!(first.contains("command `tasks-desktop` removed"), "{first}");
        assert!(!starter_files(&dir, "tasks-desktop")[0].exists());
        assert!(starter_files(&dir, "rules-desktop")[0].exists());
        assert!(path_holds(
            &store.value.borrow().clone().unwrap().0,
            &dir,
            &env
        ));

        let last = uninstall("rules-desktop", &env, Some(&store)).unwrap();
        assert!(last.contains("off the PATH"), "{last}");
        assert!(!dir.exists());
        let (value, expandable) = store.value.borrow().clone().unwrap();
        assert_eq!(value, r"C:\keep");
        assert!(expandable);

        // Nothing there: nothing to say, no error.
        assert_eq!(uninstall("rules-desktop", &env, Some(&store)).unwrap(), "");
    }

    #[test]
    fn without_a_store_only_the_path_of_the_process_is_looked_at() {
        let root = Scratch::new("cli-entry-nostore");
        let mut pairs = vec![
            (
                family::FAMILY_DIR_VAR.to_owned(),
                root.join("family").into_os_string(),
            ),
            (
                "LOCALAPPDATA".to_owned(),
                root.join("local").into_os_string(),
            ),
        ];
        let off = Env::from_pairs(pairs.clone());
        let said = install("x-desktop", &root.join("x.exe"), &off, None).unwrap();
        assert!(said.contains("is not on PATH"), "{said}");
        let dir = starter_dir(&off).unwrap();
        pairs.push(("PATH".to_owned(), std::env::join_paths([&dir]).unwrap()));
        let on = Env::from_pairs(pairs);
        let said = install("x-desktop", &root.join("x.exe"), &on, None).unwrap();
        assert!(!said.contains("not on PATH"), "{said}");
    }

    /// The real thing on Windows: cmd runs the starter, which finds its program through `%LOCALAPPDATA%`, hands every
    /// argument on and gives its exit code back; a program that is gone ends with 3.
    #[cfg(windows)]
    #[test]
    fn the_cmd_starter_runs_its_program_with_the_arguments_and_its_exit_code() {
        use std::process::Command;
        let root = Scratch::new("cli-entry-run");
        let env = scratch_env(&root);
        let system = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap();
        let cmd = system.join("System32").join("cmd.exe");
        let program = root
            .join("local")
            .join("fake-app")
            .join("bin")
            .join("fake.exe");
        std::fs::create_dir_all(program.parent().unwrap()).unwrap();
        std::fs::copy(&cmd, &program).unwrap();
        install("fake-app", &program, &env, None).unwrap();
        let starter = starter_files(&starter_dir(&env).unwrap(), "fake-app")[0].clone();
        assert!(cmd_text("fake-app", &program, &env)
            .unwrap()
            .contains("%LOCALAPPDATA%"));

        let run = |args: &[&str]| {
            Command::new(&cmd)
                .arg("/d")
                .arg("/c")
                .arg(&starter)
                .args(args)
                .env("LOCALAPPDATA", root.join("local"))
                .output()
                .unwrap()
        };
        let code = run(&["/d", "/c", "exit", "7"]);
        assert_eq!(code.status.code(), Some(7), "{code:?}");
        let echo = run(&["/d", "/c", "echo", "one", "two"]);
        assert_eq!(echo.status.code(), Some(0), "{echo:?}");
        assert!(
            String::from_utf8_lossy(&echo.stdout).contains("one two"),
            "{echo:?}"
        );

        // PowerShell finds the command by its name in the folder on PATH (the `.cmd`, not the shell script beside
        // it) and waits for it, so `$LASTEXITCODE` is the exit code of the program.
        let dir = starter_dir(&env).unwrap();
        let path = std::env::join_paths([dir.clone(), system.join("System32")]).unwrap();
        let shell = system
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe");
        let by_name = Command::new(&shell)
            .args(["-NoProfile", "-NonInteractive", "-Command"])
            .arg("fake-app /d /c exit 5; exit $LASTEXITCODE")
            .env("PATH", &path)
            .env("LOCALAPPDATA", root.join("local"))
            .output()
            .unwrap();
        assert_eq!(by_name.status.code(), Some(5), "{by_name:?}");

        // Git Bash runs the shell script by its name, when Git for Windows is there.
        let git_sh = std::env::var_os("ProgramFiles")
            .map(|p| PathBuf::from(p).join("Git").join("bin").join("sh.exe"))
            .filter(|p| p.is_file());
        match git_sh {
            Some(sh) => {
                let bash = Command::new(&sh)
                    .arg("-c")
                    .arg("PATH=\"$(cygpath -u \"$1\"):$PATH\"; fake-app /d /c exit 6")
                    .arg("sh")
                    .arg(&dir)
                    // MSYS would read the switches `/d` `/c` of cmd as paths (`D:/`); the hubs take no such words.
                    .env("MSYS2_ARG_CONV_EXCL", "*")
                    .output()
                    .unwrap();
                assert_eq!(bash.status.code(), Some(6), "{bash:?}");
            }
            None => println!("n/a  Git Bash: no Git for Windows here"),
        }

        std::fs::remove_file(&program).unwrap();
        let gone = run(&["x"]);
        assert_eq!(gone.status.code(), Some(PROGRAM_GONE), "{gone:?}");
        assert!(
            String::from_utf8_lossy(&gone.stderr).contains("its program is gone"),
            "{gone:?}"
        );
    }
}
