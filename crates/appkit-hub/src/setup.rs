//! The first start: where the vault is, where Neovim is, and whether to install the program.
//!
//! **In the page** after a double click: the hub starts without asking anything, and its page shows what
//! is missing (the vault, Neovim) and offers the installation. The page names no path: the folder and the
//! program are chosen in the dialog of the system that the hub shows ([`crate::editor::pick_program`],
//! the folder dialog of the opener), and the hub checks what was chosen ([`checked_vault`],
//! [`checked_nvim`]) before it keeps it. The installation runs as `<program> install --open` of its own
//! ([`start_install`]): it stops this hub, copies the program, starts the hub again from the copy and opens
//! its page (the sessions of the page end with the hub that was stopped).
//!
//! **In the terminal** as the second way: a console program started from a terminal (both ends at it;
//! the debug build, every program on macOS and Linux) asks as before, and `<program> setup` asks
//! always. A window program (the release build of a hub on Windows, see `appkit_core::console`) shares
//! the terminal with a shell that does not wait for it: for `setup` [`Console`] opens a console window
//! of its own, only when there is something to ask, and closes it afterwards. A double click never gets
//! a console window: [`interactive`] is false for it, as for a start without a person (a pipe, a
//! script, a service).
//!
//! Each question is asked only once: what was answered (or declined) is remembered in `setup.json` in
//! the config folder, so a second start asks nothing. The questions change no file but the settings of
//! the app, the install folder of the user and, when it is said yes to, the login entry and the desktop
//! link (everything `<program> install` does, which this calls).

use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use appkit_core::console;
use serde::{Deserialize, Serialize};

use crate::background::{self, Background, InstallOptions};
use crate::config::{self, AppSettings, Env, FileConfig};
use crate::instance;

/// What the questions need to know about the app.
#[derive(Debug, Clone, Copy)]
pub struct SetupApp<'a> {
    pub background: &'a Background<'a>,
    /// The variable that names the vault (`TASKS_VAULT`): a vault that it names is not asked for.
    pub vault_var: &'a str,
    /// What is said before the vault is asked for: what a vault is, in the words of the app.
    pub vault_explained: &'a str,
}

impl SetupApp<'_> {
    fn identity(&self) -> &appkit_core::identity::AppIdentity {
        self.background.login.identity
    }
}

/// Where the questions go and the answers come from. The console in the program; a script in the tests.
pub trait Io {
    /// Show `question` and read one line. `None` when there is no more input (the end of a pipe).
    fn ask(&mut self, question: &str) -> Option<String>;
    /// Tell something.
    fn say(&mut self, text: &str);
    /// The questions are over (`finished`: without an error). The console closes a window it opened for them.
    fn done(&mut self, finished: bool) {
        let _ = finished;
    }
}

/// The console of the process, or a console window of its own when the process is a window program (no
/// console, or the console of a terminal whose shell reads the same keys).
pub struct Console;

impl Console {
    /// A console to ask in; false when there is none and none could be opened.
    fn ready() -> bool {
        if !needs_window(console::attach_parent(), console::started_without_streams()) {
            return true;
        }
        if console::shares_terminal() {
            println!("The questions of the first start are asked in a window of their own.");
        }
        let title = std::env::current_exe()
            .ok()
            .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_default();
        console::open_own(&format!("{title}: first start"))
    }
}

impl Io for Console {
    fn ask(&mut self, question: &str) -> Option<String> {
        if !Self::ready() {
            return None;
        }
        print!("{question} ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        match std::io::stdin().lock().read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(line.trim().to_owned()),
        }
    }

    fn say(&mut self, text: &str) {
        if Self::ready() {
            println!("{text}");
        }
    }

    fn done(&mut self, finished: bool) {
        if !console::opened_own() {
            return;
        }
        // The window closes with the program's last word: what was installed is read before that.
        if finished {
            print!("Press Enter to close this window.");
            let _ = std::io::stdout().flush();
            let _ = std::io::stdin().lock().read_line(&mut String::new());
        }
        console::close_own();
    }
}

/// With this set, the questions are asked although there is no terminal (the tests pipe the answers in).
fn assume_terminal(app: &SetupApp<'_>) -> String {
    app.identity().var("ASSUME_TERMINAL")
}
/// With this set, nothing is asked, whatever the console is.
fn no_setup(app: &SetupApp<'_>) -> String {
    app.identity().var("NO_SETUP")
}

/// Whether there is a person to ask.
#[must_use]
pub fn interactive(app: &SetupApp<'_>, env: &Env) -> bool {
    if env.get(&no_setup(app)).is_some() {
        return false;
    }
    env.get(&assume_terminal(app)).is_some()
        || person_present(
            console::attach_parent(),
            std::io::stdin().is_terminal() && std::io::stdout().is_terminal(),
        )
}

/// Whether the questions need a window of their own: in the terminal's console the shell reads the same keys,
/// and a program started without streams has nowhere to ask. Streams that were given (answers piped in with
/// `<PREFIX>_ASSUME_TERMINAL`) are asked over, as a console program asks over its console.
fn needs_window(console: console::Console, without_streams: bool) -> bool {
    match console {
        console::Console::Own => false,
        console::Console::Parent => true,
        console::Console::None => without_streams,
    }
}

/// Whether the questions are asked in a console without being asked for (`<program> setup`): only in a console
/// of the program's own with both ends at a terminal (with the output in a file or a pipe the questions would go
/// where nobody reads them). A window program in the terminal's console shares it with the shell, and a double
/// click (no console, no streams) gets the page: neither opens a console window for the questions.
fn person_present(console: console::Console, terminal: bool) -> bool {
    match console {
        console::Console::Own => terminal,
        console::Console::Parent | console::Console::None => false,
    }
}

/// What was asked already. A missing or unreadable file is "nothing yet": the worst case is a question twice.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Asked {
    vault: bool,
    install_declined: bool,
}

fn asked_path(dir: &Path) -> PathBuf {
    dir.join("setup.json")
}

fn load_asked(dir: &Path) -> Asked {
    std::fs::read_to_string(asked_path(dir))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_asked(dir: &Path, asked: &Asked) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot make the config folder: {e}"))?;
    let text = serde_json::to_string_pretty(asked).map_err(|e| e.to_string())?;
    std::fs::write(asked_path(dir), format!("{text}\n"))
        .map_err(|e| format!("cannot write the setup notes: {e}"))
}

/// A yes or a no; an empty line (or anything else) is the default.
fn yes_no(io: &mut dyn Io, question: &str, default_yes: bool) -> bool {
    let hint = if default_yes { "[Y/n]" } else { "[y/N]" };
    match io.ask(&format!("{question} {hint}")) {
        Some(answer) => match answer.to_lowercase().as_str() {
            "y" | "yes" | "j" | "ja" => true,
            "n" | "no" | "nein" => false,
            _ => default_yes,
        },
        None => false,
    }
}

/// How often a wrong folder is asked again.
const VAULT_TRIES: usize = 3;

/// Ask for the vault folder when none is set (settings or the variable of the app), and write it to the settings.
/// An empty line skips; the page then says what is missing.
fn ask_vault<A: AppSettings>(
    app: &SetupApp<'_>,
    io: &mut dyn Io,
    env: &Env,
    dir: &Path,
) -> Result<(), String> {
    let file: FileConfig<A> = config::load_file(dir).map_err(|e| e.to_string())?;
    // What the hub would read: the main vault, or the active one of the named vaults.
    if file.effective().vault.is_some() || env.path(app.vault_var).is_some() {
        return Ok(());
    }
    io.say(app.vault_explained);
    for _ in 0..VAULT_TRIES {
        let Some(answer) = io.ask("Path of your vault (empty line: later):") else {
            return Ok(());
        };
        let answer = answer.trim().trim_matches('"').trim();
        if answer.is_empty() {
            return Ok(());
        }
        // The console's folder means something to the person, the hub's does not: keep the whole path.
        let path = match checked_vault(answer, std::env::current_dir().ok().as_deref()) {
            Ok(path) => path,
            Err(VaultRefusal::NotAFolder(path)) => {
                io.say(&format!("{} is not a folder.", path.display()));
                continue;
            }
            Err(refused) => {
                io.say(&format!("{}.", capitalised(refused.for_page())));
                continue;
            }
        };
        // The question can take minutes: the file is read again under its lock, so a change another writer made
        // in the meantime stays.
        config::update_file::<A>(dir, |file| {
            file.vault = Some(path.clone());
            Ok(())
        })?;
        io.say(&format!(
            "The vault is {} (kept in the settings).",
            path.display()
        ));
        return Ok(());
    }
    io.say(&format!(
        "No vault set. Set it later in the settings file or with {}.",
        app.vault_var
    ));
    Ok(())
}

/// The text with its first letter in upper case (a text of the page said in the console).
fn capitalised(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// Why a folder is not taken as the vault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultRefusal {
    /// A network folder (`\\host\share`): not looked at, since opening it makes Windows sign in to that host.
    Remote,
    /// A relative path with no folder to take it from.
    Relative,
    /// Nothing, or a file, at this path.
    NotAFolder(PathBuf),
}

impl VaultRefusal {
    /// A short text for the page: no path.
    #[must_use]
    pub fn for_page(&self) -> &'static str {
        match self {
            Self::Remote => "a network folder is not accepted: use a drive or a mounted folder",
            Self::Relative => "the folder must be given with its whole path",
            Self::NotAFolder(_) => "there is no such folder",
        }
    }
}

/// The folder `answer` names as the vault, checked: not a network folder (looked at before anything touches
/// it), a folder that is there, with its whole path. A relative answer is taken from `base` (the folder of the
/// console); without one it is refused (the page always names a whole path).
///
/// # Errors
///
/// Why the folder is not taken.
pub fn checked_vault(answer: &str, base: Option<&Path>) -> Result<PathBuf, VaultRefusal> {
    if config::is_remote_path(answer) {
        return Err(VaultRefusal::Remote);
    }
    let typed = PathBuf::from(answer);
    let path = if typed.is_absolute() {
        typed
    } else {
        base.ok_or(VaultRefusal::Relative)?.join(&typed)
    };
    if config::is_remote_path(&path.to_string_lossy()) {
        return Err(VaultRefusal::Remote);
    }
    if !path.is_dir() {
        return Err(VaultRefusal::NotAFolder(path));
    }
    Ok(path)
}

/// Whether the file is the program of Neovim by its name (`nvim`, `nvim.exe`; case does not count on Windows).
fn named_like_nvim(path: &Path) -> bool {
    let wanted = crate::editor::nvim_file_name();
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|name| {
            if cfg!(windows) {
                name.eq_ignore_ascii_case(wanted)
            } else {
                name == wanted
            }
        })
}

/// Check a Neovim the person chose in the dialog before it is kept: a file named as the program of Neovim
/// (nothing else is started to ask its version), not on a network folder, that answers `--version` with a
/// version the engine supports. Returns that version.
///
/// # Errors
///
/// A short text for the page, no path: what is wrong with the choice.
pub async fn checked_nvim(path: &Path, env: &Env) -> Result<(u32, u32, u32), String> {
    if config::is_remote_path(&path.to_string_lossy()) {
        return Err("a program on a network folder is not accepted".to_owned());
    }
    if !named_like_nvim(path) {
        return Err(format!(
            "choose the program {} of Neovim",
            crate::editor::nvim_file_name()
        ));
    }
    crate::discovery::check_nvim(path, env)
        .await
        .map_err(|problem| match problem {
            crate::discovery::SetupProblem::NvimTooOld { found: (a, b, c) } => {
                let (x, y, z) = crate::discovery::MIN_NVIM;
                format!("this Neovim is {a}.{b}.{c}; {x}.{y}.{z} or newer is needed")
            }
            crate::discovery::SetupProblem::NvimNotFound { .. } => {
                "there is no such program".to_owned()
            }
            _ => "this program did not answer as Neovim does".to_owned(),
        })
}

/// What the page offers about the installation of the program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallOffer {
    /// The hub runs from the installed copy, or from a file with the same bytes: nothing to offer.
    Installed,
    /// Nothing is installed: install (with the link and the login entry, when chosen).
    Install,
    /// Another version is installed: replace it with this one.
    Replace,
    /// The person said no once: the page does not offer it again by itself.
    Declined,
}

/// The state of the installation, from where this program runs and what the install folder holds.
fn install_state(
    app: &SetupApp<'_>,
    env: &Env,
    asked: &Asked,
) -> Result<(InstallOffer, PathBuf), String> {
    let target = background::installed_program(app.background, env)?;
    let from = std::env::current_exe().map_err(|e| format!("own program not found: {e}"))?;
    if matches!(
        (std::fs::canonicalize(&from), std::fs::canonicalize(&target)),
        (Ok(a), Ok(b)) if a == b
    ) || (target.is_file() && same_bytes(&from, &target))
    {
        return Ok((InstallOffer::Installed, target));
    }
    if asked.install_declined {
        return Ok((InstallOffer::Declined, target));
    }
    let offer = if target.is_file() {
        InstallOffer::Replace
    } else {
        InstallOffer::Install
    };
    Ok((offer, target))
}

/// What the page offers about the installation now.
///
/// # Errors
///
/// No install folder, or the own program is not found.
pub fn install_offer(app: &SetupApp<'_>, env: &Env) -> Result<InstallOffer, String> {
    let dir = instance::config_folder(env, app.identity())?;
    install_state(app, env, &load_asked(&dir)).map(|(offer, _)| offer)
}

/// The person said no to the installation in the page: it is not offered again (nor asked in the console).
///
/// # Errors
///
/// The note cannot be written.
pub fn decline_install(app: &SetupApp<'_>, env: &Env) -> Result<(), String> {
    let dir = instance::config_folder(env, app.identity())?;
    let mut asked = load_asked(&dir);
    asked.install_declined = true;
    save_asked(&dir, &asked)
}

/// The vault was chosen (in the page): the console does not ask for it again.
///
/// # Errors
///
/// The note cannot be written.
pub fn note_vault_chosen(app: &SetupApp<'_>, env: &Env) -> Result<(), String> {
    let dir = instance::config_folder(env, app.identity())?;
    let mut asked = load_asked(&dir);
    asked.vault = true;
    save_asked(&dir, &asked)
}

/// Forget what was asked and declined, so that `<program> setup` asks again.
///
/// # Errors
///
/// The note cannot be removed.
pub fn forget_asked(app: &SetupApp<'_>, env: &Env) -> Result<(), String> {
    let dir = instance::config_folder(env, app.identity())?;
    match std::fs::remove_file(asked_path(&dir)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("cannot remove the setup notes: {e}")),
    }
}

/// The arguments of `<program> install` for what the person chose in the page. `--open`: the program opens the page
/// of the hub it started again, with a pairing link of its own (the sessions of the page end with the hub that is
/// stopped for the copy), so an app whose first start is in the page takes `install --open`.
#[must_use]
pub fn install_args(autostart: bool, shortcut: bool) -> Vec<&'static str> {
    let mut args = vec!["install", "--open"];
    if autostart {
        args.push("--autostart");
    }
    if shortcut {
        args.push("--shortcut");
    }
    args
}

/// Start `<program> install --open` as a process of its own, from the program this hub runs from: it stops this hub,
/// copies the program to the install folder, starts the hub again from there (what it does in a terminal) and opens
/// its page.
/// It is not a child of the hub: it outlives it. `<PREFIX>_INSTALL_LOG` (a file) is the seam of the tests: the
/// arguments are written there instead.
///
/// # Errors
///
/// The program could not be started (or the log not written).
pub fn start_install(
    app: &SetupApp<'_>,
    env: &Env,
    autostart: bool,
    shortcut: bool,
) -> Result<(), String> {
    let args = install_args(autostart, shortcut);
    if let Some(log) = env.path(&app.identity().var("INSTALL_LOG")) {
        let line = format!(
            "{}
",
            args.join(" ")
        );
        return std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log)
            .and_then(|mut out| out.write_all(line.as_bytes()))
            .map_err(|_| "the test log cannot be written".to_owned());
    }
    let program = std::env::current_exe().map_err(|e| format!("own program not found: {e}"))?;
    let cwd = program
        .parent()
        .ok_or_else(|| "the program has no folder".to_owned())?
        .to_path_buf();
    crate::launch::spawn(
        &program,
        &args,
        &crate::launch::LaunchOpts {
            cwd: &cwd,
            window: crate::launch::Window::Default,
        },
    )
    .map(|_pid| ())
}

/// Whether two files hold the same bytes.
fn same_bytes(a: &Path, b: &Path) -> bool {
    matches!((std::fs::read(a), std::fs::read(b)), (Ok(x), Ok(y)) if x == y)
}

/// The install question: install, replace an older copy, or leave it. Returns the program to start the hub from
/// when it is the installed copy.
async fn ask_install(
    app: &SetupApp<'_>,
    io: &mut dyn Io,
    env: &Env,
    asked: &mut Asked,
) -> Result<Option<PathBuf>, String> {
    let target = background::installed_program(app.background, env)?;
    let from = std::env::current_exe().map_err(|e| format!("own program not found: {e}"))?;
    if matches!(
        (std::fs::canonicalize(&from), std::fs::canonicalize(&target)),
        (Ok(a), Ok(b)) if a == b
    ) {
        return Ok(None); // this is the installed copy
    }
    let folder = target
        .parent()
        .map_or_else(String::new, |p| p.display().to_string());
    if target.is_file() {
        if install_state(app, env, asked)?.0 == InstallOffer::Installed {
            return Ok(Some(target));
        }
        if asked.install_declined
            || !yes_no(
                io,
                &format!("A different version is installed in {folder}. Replace it with this one?"),
                true,
            )
        {
            // Once is enough: the next double click does not ask the same question again.
            asked.install_declined = true;
            return Ok(None);
        }
        return Ok(install_or_warn(app, io, InstallOptions::default(), target).await);
    }
    if asked.install_declined {
        return Ok(None);
    }
    if !yes_no(
        io,
        &format!("Install the program to {folder} (no administrator rights needed)?"),
        true,
    ) {
        asked.install_declined = true;
        return Ok(None);
    }
    let opts = InstallOptions {
        autostart: yes_no(io, "Start it when you log in?", false),
        shortcut: yes_no(
            io,
            "Put a link on the desktop as well (the Start menu has one anyway)?",
            true,
        ),
        ..InstallOptions::default()
    };
    Ok(install_or_warn(app, io, opts, target).await)
}

/// Install, and say what was done. A step that fails (the link, the login entry) is told and does not stop the
/// double click: the hub still starts, from the program that was clicked.
async fn install_or_warn(
    app: &SetupApp<'_>,
    io: &mut dyn Io,
    opts: InstallOptions,
    target: PathBuf,
) -> Option<PathBuf> {
    match background::install(app.background, opts).await {
        Ok(text) => {
            io.say(&text);
            Some(target)
        }
        Err(e) => {
            io.say(&format!(
                "The installation did not finish: {e}\nThe hub starts from this program anyway; `{} install` tries again.",
                app.identity().binary
            ));
            None
        }
    }
}

/// An [`Io`] that notes when the input ran out (Ctrl-Z, a closed console): an answer that was never given is
/// not remembered as one.
struct Watch<'a> {
    inner: &'a mut dyn Io,
    ended: bool,
}

impl Io for Watch<'_> {
    fn ask(&mut self, question: &str) -> Option<String> {
        let answer = self.inner.ask(question);
        self.ended |= answer.is_none();
        answer
    }

    fn say(&mut self, text: &str) {
        self.inner.say(text);
    }
}

/// The questions of a first start. Returns the installed program to start the hub from, when there is one.
///
/// # Errors
///
/// A text for the person at the console (the settings cannot be read or written, the install failed).
pub async fn run<A: AppSettings>(
    app: &SetupApp<'_>,
    io: &mut dyn Io,
    env: &Env,
) -> Result<Option<PathBuf>, String> {
    let result = ask_all::<A>(app, io, env).await;
    io.done(result.is_ok());
    result
}

async fn ask_all<A: AppSettings>(
    app: &SetupApp<'_>,
    io: &mut dyn Io,
    env: &Env,
) -> Result<Option<PathBuf>, String> {
    let dir = instance::config_folder(env, app.identity())?;
    let mut asked = load_asked(&dir);
    let mut io = Watch {
        inner: io,
        ended: false,
    };
    if !asked.vault {
        ask_vault::<A>(app, &mut io, env, &dir)?;
        if io.ended {
            return Ok(None); // nothing was answered: ask again next time
        }
        asked.vault = true;
        save_asked(&dir, &asked)?;
    }
    let program = ask_install(app, &mut io, env, &mut asked).await?;
    if !io.ended {
        save_asked(&dir, &asked)?;
    }
    Ok(program)
}

/// The settings as they are now (for the tests of the vault step).
#[must_use]
pub fn vault_in_settings<A: AppSettings>(dir: &Path) -> Option<PathBuf> {
    config::load_file(dir)
        .ok()
        .and_then(|f: FileConfig<A>| f.vault)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::NoSettings;
    use crate::loginitems::LoginApp;
    use appkit_core::identity::AppIdentity;
    use appkit_scratch::Scratch;

    /// An app that is not the one this kit was made from.
    const IDENTITY: AppIdentity = AppIdentity {
        app: "rules-desktop",
        binary: "rules-hub",
        env_prefix: "RULES_DESKTOP",
        title: "Rules",
        default_port: 17616,
        route_ns: "rules",
    };
    const BACKGROUND: Background<'static> = Background {
        login: LoginApp {
            identity: &IDENTITY,
            summary: "the rule overview",
        },
        extra_files: &[],
        uninstall_script: "scripts/rules-uninstall",
    };
    const APP: SetupApp<'static> = SetupApp {
        background: &BACKGROUND,
        vault_var: "RULES_VAULT",
        vault_explained: "No rule vault is set yet. It is the folder of your rule sets.",
    };

    type Cfg = FileConfig<NoSettings>;

    fn ask_vault(io: &mut dyn Io, env: &Env, dir: &Path) -> Result<(), String> {
        super::ask_vault::<NoSettings>(&APP, io, env, dir)
    }
    async fn run(io: &mut dyn Io, env: &Env) -> Result<Option<PathBuf>, String> {
        super::run::<NoSettings>(&APP, io, env).await
    }
    fn interactive(env: &Env) -> bool {
        super::interactive(&APP, env)
    }
    fn vault_in_settings(dir: &Path) -> Option<PathBuf> {
        super::vault_in_settings::<NoSettings>(dir)
    }
    fn no_setup() -> String {
        super::no_setup(&APP)
    }
    fn assume_terminal() -> String {
        super::assume_terminal(&APP)
    }

    /// Answers from a list, and what was asked and said.
    struct Script {
        answers: Vec<&'static str>,
        asked: Vec<String>,
        said: Vec<String>,
        done: Vec<bool>,
    }

    impl Script {
        fn new(answers: &[&'static str]) -> Self {
            let mut answers = answers.to_vec();
            answers.reverse();
            Self {
                answers,
                asked: Vec::new(),
                said: Vec::new(),
                done: Vec::new(),
            }
        }
    }

    impl Io for Script {
        fn ask(&mut self, question: &str) -> Option<String> {
            self.asked.push(question.to_owned());
            self.answers.pop().map(str::to_owned)
        }
        fn say(&mut self, text: &str) {
            self.said.push(text.to_owned());
        }
        fn done(&mut self, finished: bool) {
            self.done.push(finished);
        }
    }

    /// Only a console of the program's own with both ends at the terminal is asked in without `setup`: a double
    /// click (no console, no streams) and a window program in the terminal's console get the page, so no console
    /// window is opened for them.
    #[test]
    fn only_a_console_of_its_own_at_a_terminal_is_asked_in_and_a_double_click_gets_the_page() {
        use appkit_core::console::Console as C;
        for (console, terminal, expected) in [
            (C::Own, true, true),
            (C::Own, false, false),
            (C::Parent, true, false),
            (C::Parent, false, false),
            (C::None, false, false),
        ] {
            assert_eq!(
                person_present(console, terminal),
                expected,
                "{console:?} terminal={terminal}"
            );
        }
    }

    #[test]
    fn a_folder_for_the_vault_is_checked_before_it_is_kept() {
        let vault = temp("checked-vault");
        assert_eq!(
            checked_vault(&vault.display().to_string(), None),
            Ok(vault.to_path_buf())
        );
        // The page names whole paths only; the console takes a relative one from its folder.
        assert_eq!(checked_vault("sub", None), Err(VaultRefusal::Relative));
        std::fs::create_dir(vault.join("sub")).unwrap();
        assert_eq!(checked_vault("sub", Some(&vault)), Ok(vault.join("sub")));
        let share = format!("{0}{0}no-such-host{0}share", '\\');
        assert_eq!(checked_vault(&share, None), Err(VaultRefusal::Remote));
        let gone = vault.join("gone");
        assert_eq!(
            checked_vault(&gone.display().to_string(), None),
            Err(VaultRefusal::NotAFolder(gone))
        );
        let file = vault.join("a.md");
        std::fs::write(&file, "x").unwrap();
        assert!(checked_vault(&file.display().to_string(), None).is_err());
        // The texts for the page name no path.
        for refused in [
            VaultRefusal::Remote,
            VaultRefusal::Relative,
            VaultRefusal::NotAFolder(file),
        ] {
            assert!(!refused.for_page().contains(&*vault.display().to_string()));
        }
    }

    /// Only a file named as the program of Neovim is started to ask its version: a program the person chose by
    /// mistake (or that a page would like to run) is refused before it runs.
    #[tokio::test]
    async fn a_chosen_neovim_is_refused_unless_it_is_named_so_and_there() {
        let dir = temp("checked-nvim");
        let other = dir.join(if cfg!(windows) { "calc.exe" } else { "calc" });
        std::fs::write(&other, "not run").unwrap();
        let env = Env::from_pairs([("A", "b")]);
        let refused = checked_nvim(&other, &env).await.unwrap_err();
        assert!(
            refused.contains(crate::editor::nvim_file_name()),
            "{refused}"
        );
        let missing = dir.join(crate::editor::nvim_file_name());
        assert_eq!(
            checked_nvim(&missing, &env).await.unwrap_err(),
            "there is no such program"
        );
        let share = format!("{0}{0}no-such-host{0}share{0}nvim.exe", '\\');
        assert!(checked_nvim(Path::new(&share), &env)
            .await
            .unwrap_err()
            .contains("network"));
        // A file of that name that is no program does not answer as Neovim does.
        std::fs::write(&missing, "not a program").unwrap();
        let broken = checked_nvim(&missing, &env).await.unwrap_err();
        assert_eq!(broken, "this program did not answer as Neovim does");
        assert!(named_like_nvim(Path::new(if cfg!(windows) {
            "C:/x/NVIM.EXE"
        } else {
            "/x/nvim"
        })));
    }

    #[test]
    fn the_installation_is_offered_until_it_is_done_or_declined() {
        let (env, config, bin) = run_env("offer");
        assert_eq!(install_offer(&APP, &env).unwrap(), InstallOffer::Install);
        std::fs::write(bin.join(IDENTITY.program_file()), "an older program").unwrap();
        assert_eq!(install_offer(&APP, &env).unwrap(), InstallOffer::Replace);
        // The same bytes as this program: installed.
        std::fs::copy(
            std::env::current_exe().unwrap(),
            bin.join(IDENTITY.program_file()),
        )
        .unwrap();
        assert_eq!(install_offer(&APP, &env).unwrap(), InstallOffer::Installed);
        std::fs::remove_file(bin.join(IDENTITY.program_file())).unwrap();

        decline_install(&APP, &env).unwrap();
        assert_eq!(install_offer(&APP, &env).unwrap(), InstallOffer::Declined);
        assert!(
            load_asked(&config).install_declined,
            "the console does not ask either"
        );
        note_vault_chosen(&APP, &env).unwrap();
        let asked = load_asked(&config);
        assert!(asked.vault && asked.install_declined);
        // `setup` asks again.
        forget_asked(&APP, &env).unwrap();
        forget_asked(&APP, &env).unwrap();
        assert_eq!(install_offer(&APP, &env).unwrap(), InstallOffer::Install);
    }

    #[test]
    fn the_installation_of_the_page_is_the_install_command_with_what_was_chosen() {
        assert_eq!(install_args(false, false), ["install", "--open"]);
        assert_eq!(
            install_args(true, true),
            ["install", "--open", "--autostart", "--shortcut"]
        );
        assert_eq!(
            install_args(false, true),
            ["install", "--open", "--shortcut"]
        );
        let (env, config, _bin) = run_env("install-log");
        let log = config.join("install.log");
        let mut pairs: Vec<(String, String)> = vec![(
            "RULES_DESKTOP_INSTALL_LOG".to_owned(),
            log.display().to_string(),
        )];
        pairs.extend(
            ["RULES_DESKTOP_CONFIG_DIR", "RULES_DESKTOP_INSTALL_DIR"]
                .iter()
                .filter_map(|k| {
                    env.get(k)
                        .map(|v| ((*k).to_owned(), v.to_string_lossy().into_owned()))
                }),
        );
        let env = Env::from_pairs(pairs);
        start_install(&APP, &env, true, false).unwrap();
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            "install --open --autostart\n"
        );
    }

    /// Answers piped in (a spec with `ASSUME_TERMINAL`, also of a release build) are read from the pipe; the
    /// terminal's console and a start without streams get a window.
    #[test]
    fn only_the_terminal_and_a_start_without_streams_need_a_window() {
        use appkit_core::console::Console as C;
        assert!(!needs_window(C::Own, false));
        assert!(needs_window(C::Parent, false));
        assert!(needs_window(C::None, true));
        assert!(!needs_window(C::None, false), "streams that were given");
    }

    fn temp(name: &str) -> Scratch {
        Scratch::new(&format!("hub-setup-{name}"))
    }

    #[test]
    fn a_yes_or_no_reads_the_answer_and_falls_back_to_the_default() {
        for (answer, default, expected) in [
            ("y", false, true),
            ("Yes", false, true),
            ("ja", false, true),
            ("n", true, false),
            ("NO", true, false),
            ("", true, true),
            ("", false, false),
            ("maybe", true, true),
        ] {
            let mut io = Script::new(&[answer]);
            assert_eq!(yes_no(&mut io, "q", default), expected, "{answer:?}");
        }
        // The end of the input is a no, never a yes.
        assert!(!yes_no(&mut Script::new(&[]), "q", true));
    }

    #[test]
    fn the_vault_is_asked_for_checked_and_kept_in_the_settings() {
        let dir = temp("vault");
        let vault = temp("vault-folder");
        let env = Env::from_pairs([("RULES_DESKTOP_CONFIG_DIR", dir.display().to_string())]);
        let wrong = dir.join("nothing-here").display().to_string();
        let right = vault.display().to_string();
        let mut io = Script::new(&[
            Box::leak(wrong.into_boxed_str()),
            Box::leak(right.into_boxed_str()),
        ]);
        ask_vault(&mut io, &env, &dir).unwrap();
        assert_eq!(io.asked.len(), 2, "a wrong folder is asked again");
        assert!(io.said.iter().any(|s| s.contains("is not a folder")));
        assert_eq!(vault_in_settings(&dir), Some(vault.to_path_buf()));
    }

    #[test]
    fn an_empty_line_skips_and_a_known_vault_is_not_asked() {
        let dir = temp("vault-skip");
        let env = Env::from_pairs([("RULES_DESKTOP_CONFIG_DIR", dir.display().to_string())]);
        let mut io = Script::new(&[""]);
        ask_vault(&mut io, &env, &dir).unwrap();
        assert_eq!(vault_in_settings(&dir), None);

        let known = Env::from_pairs([
            ("RULES_DESKTOP_CONFIG_DIR", dir.display().to_string()),
            ("RULES_VAULT", dir.display().to_string()),
        ]);
        let mut none = Script::new(&[]);
        ask_vault(&mut none, &known, &dir).unwrap();
        assert!(none.asked.is_empty(), "RULES_VAULT is a vault");
    }

    #[test]
    fn three_wrong_folders_end_the_question() {
        let dir = temp("vault-three");
        let env = Env::from_pairs([("RULES_DESKTOP_CONFIG_DIR", dir.display().to_string())]);
        let mut io = Script::new(&["/no/1", "/no/2", "/no/3", "/no/4"]);
        ask_vault(&mut io, &env, &dir).unwrap();
        assert_eq!(io.asked.len(), VAULT_TRIES);
        assert_eq!(vault_in_settings(&dir), None);
        // The person is told what a vault is in the words of the app, and where to set it later.
        assert_eq!(
            io.said.first().map(String::as_str),
            Some(APP.vault_explained)
        );
        let last = io.said.last().unwrap();
        assert!(last.contains("RULES_VAULT"), "{last}");
        assert!(!last.to_lowercase().contains("tasks"), "{last}");
    }

    #[test]
    fn what_was_asked_is_remembered_and_a_broken_note_is_nothing_yet() {
        let dir = temp("asked");
        assert!(!load_asked(&dir).vault);
        save_asked(
            &dir,
            &Asked {
                vault: true,
                install_declined: true,
            },
        )
        .unwrap();
        let back = load_asked(&dir);
        assert!(back.vault && back.install_declined);
        std::fs::write(asked_path(&dir), "{ not json").unwrap();
        assert!(!load_asked(&dir).vault);
    }

    #[test]
    fn nothing_is_asked_without_a_person_or_when_it_is_switched_off() {
        // The names are spelt out here: a helper that builds them from the same code as the kit
        // would stay green with a prefix that is hard-coded in the kit.
        assert_eq!(no_setup(), "RULES_DESKTOP_NO_SETUP");
        assert_eq!(assume_terminal(), "RULES_DESKTOP_ASSUME_TERMINAL");
        let off = Env::from_pairs([
            ("RULES_DESKTOP_NO_SETUP", "1"),
            ("RULES_DESKTOP_ASSUME_TERMINAL", "1"),
        ]);
        assert!(!interactive(&off));
        let piped = Env::from_pairs([("RULES_DESKTOP_ASSUME_TERMINAL", "1")]);
        assert!(interactive(&piped));
    }

    #[test]
    fn a_relative_vault_is_kept_with_its_whole_path() {
        let dir = temp("vault-relative");
        let env = Env::from_pairs([("RULES_DESKTOP_CONFIG_DIR", dir.display().to_string())]);
        // `.` is a folder of the console: what the hub reads later must not depend on where it is started.
        let mut io = Script::new(&["."]);
        ask_vault(&mut io, &env, &dir).unwrap();
        let kept = vault_in_settings(&dir).expect("kept");
        assert!(kept.is_absolute(), "{}", kept.display());
    }

    #[test]
    fn a_network_folder_is_refused_before_anything_touches_it() {
        let dir = temp("vault-remote");
        let env = Env::from_pairs([("RULES_DESKTOP_CONFIG_DIR", dir.display().to_string())]);
        let share = format!("{0}{0}no-such-host{0}share", '\\');
        let mut io = Script::new(&[Box::leak(share.into_boxed_str()), ""]);
        ask_vault(&mut io, &env, &dir).unwrap();
        assert!(
            io.said.iter().any(|s| s.contains("network folder")),
            "{:?}",
            io.said
        );
        assert_eq!(vault_in_settings(&dir), None);
    }

    /// Answers like [`Script`], but another writer changes `config.json` while the question is open.
    struct WriterMeanwhile<'a> {
        inner: Script,
        dir: &'a Path,
    }

    impl Io for WriterMeanwhile<'_> {
        fn ask(&mut self, question: &str) -> Option<String> {
            config::update_file::<NoSettings>(self.dir, |file| {
                file.nvim = Some(PathBuf::from("nvim-of-the-other-writer"));
                Ok(())
            })
            .unwrap();
            self.inner.ask(question)
        }
        fn say(&mut self, text: &str) {
            self.inner.say(text);
        }
        fn done(&mut self, finished: bool) {
            self.inner.done(finished);
        }
    }

    /// The question can take minutes: a change another writer makes to `config.json` meanwhile is kept.
    #[test]
    fn a_change_made_while_the_vault_is_asked_for_is_kept() {
        let dir = temp("vault-meanwhile");
        let vault = temp("vault-meanwhile-folder");
        let env = Env::from_pairs([("RULES_DESKTOP_CONFIG_DIR", dir.display().to_string())]);
        let mut io = WriterMeanwhile {
            inner: Script::new(&[Box::leak(vault.display().to_string().into_boxed_str())]),
            dir: &dir,
        };
        ask_vault(&mut io, &env, &dir).unwrap();
        let file: Cfg = config::load_file(&dir).unwrap();
        assert_eq!(file.vault, Some(vault.to_path_buf()));
        assert_eq!(
            file.nvim,
            Some(PathBuf::from("nvim-of-the-other-writer")),
            "the other writer's change is lost"
        );
    }

    #[test]
    fn a_vault_among_the_named_vaults_counts_as_set() {
        let dir = temp("vault-named");
        let other = temp("vault-named-folder");
        let file = Cfg {
            vaults: vec![config::VaultEntry {
                name: "work".to_owned(),
                path: other.to_path_buf(),
            }],
            active: Some("work".to_owned()),
            ..Cfg::default()
        };
        config::save_file(&dir, &file).unwrap();
        let env = Env::from_pairs([("RULES_DESKTOP_CONFIG_DIR", dir.display().to_string())]);
        let mut io = Script::new(&[]);
        ask_vault(&mut io, &env, &dir).unwrap();
        assert!(
            io.asked.is_empty(),
            "the active vault is a vault: {:?}",
            io.asked
        );
    }

    /// The environment of a run: a config folder, an install folder and a vault, all of the test's own.
    fn run_env(name: &str) -> (Env, Scratch, Scratch) {
        let config = temp(&format!("run-{name}"));
        let bin = temp(&format!("run-{name}-bin"));
        let env = Env::from_pairs([
            ("RULES_DESKTOP_CONFIG_DIR", config.display().to_string()),
            ("RULES_DESKTOP_INSTALL_DIR", bin.display().to_string()),
            ("RULES_VAULT", config.display().to_string()),
        ]);
        (env, config, bin)
    }

    #[tokio::test]
    async fn an_input_that_ran_out_is_not_remembered_as_an_answer() {
        let config = temp("run-eof");
        let bin = temp("run-eof-bin");
        // No vault is set, so the first question is the vault; the input ends there.
        let env = Env::from_pairs([
            ("RULES_DESKTOP_CONFIG_DIR", config.display().to_string()),
            ("RULES_DESKTOP_INSTALL_DIR", bin.display().to_string()),
        ]);
        let mut io = Script::new(&[]);
        let program = run(&mut io, &env).await.unwrap();
        assert_eq!(program, None);
        assert_eq!(
            io.done,
            [true],
            "the console is told once that the questions are over"
        );
        assert!(
            !asked_path(&config).exists(),
            "nothing was answered, nothing is noted"
        );

        // The install question: the input ends at it, so it is not noted as declined either.
        let (env, config, _bin) = run_env("eof-install");
        let mut io = Script::new(&[]);
        assert_eq!(run(&mut io, &env).await.unwrap(), None);
        assert!(!load_asked(&config).install_declined);
    }

    #[tokio::test]
    async fn declining_the_replacement_of_another_version_is_remembered() {
        let (env, config, bin) = run_env("replace");
        // Another version is installed already (other bytes than this test program).
        std::fs::write(bin.join(IDENTITY.program_file()), "an older program").unwrap();
        let mut io = Script::new(&["n"]);
        assert_eq!(run(&mut io, &env).await.unwrap(), None);
        assert_eq!(io.asked.len(), 1, "{:?}", io.asked);
        assert!(load_asked(&config).install_declined);

        let mut again = Script::new(&[]);
        assert_eq!(run(&mut again, &env).await.unwrap(), None);
        assert!(again.asked.is_empty(), "asked again: {:?}", again.asked);
    }
}
