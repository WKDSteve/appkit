//! The first start by a double click: where the vault is, and whether to install the program.
//!
//! Asked in the console that the double click opened, one question at a time, each only once: what was
//! answered (or declined) is remembered in `setup.json` in the config folder, so a second click asks
//! nothing. Without a person at the console (a pipe, a script, a service) nothing is asked and nothing is
//! installed: [`interactive`] is false and the program only starts the hub. The questions change no file
//! but the settings of the app, the install folder of the user and, when it is said yes to, the login entry
//! and the desktop link (everything `<program> install` does, which this calls).

use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

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
}

/// The console of the process.
pub struct Console;

impl Io for Console {
    fn ask(&mut self, question: &str) -> Option<String> {
        print!("{question} ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        match std::io::stdin().lock().read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(line.trim().to_owned()),
        }
    }

    fn say(&mut self, text: &str) {
        println!("{text}");
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
    // Both ends at a terminal: with the output in a file or a pipe the questions would go where nobody reads them.
    env.get(&no_setup(app)).is_none()
        && (env.get(&assume_terminal(app)).is_some()
            || (std::io::stdin().is_terminal() && std::io::stdout().is_terminal()))
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
    let mut file: FileConfig<A> = config::load_file(dir).map_err(|e| e.to_string())?;
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
        // Looked at before anything touches it: opening a network share (`\\host\share`) makes Windows sign in to that host.
        if config::is_remote_path(answer) {
            io.say("A network folder is not accepted: use a drive or a mounted folder.");
            continue;
        }
        // The console's folder means something to the person, the hub's does not: keep the whole path.
        let typed = PathBuf::from(answer);
        let path = if typed.is_absolute() {
            typed
        } else {
            std::env::current_dir().map_or(typed.clone(), |cwd| cwd.join(&typed))
        };
        if !path.is_dir() {
            io.say(&format!("{} is not a folder.", path.display()));
            continue;
        }
        file.vault = Some(path.clone());
        config::save_file(dir, &file)?;
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
        if same_bytes(&from, &target) {
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
        shortcut: yes_no(io, "Put a link on the desktop and in the Start menu?", true),
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
    }

    impl Script {
        fn new(answers: &[&'static str]) -> Self {
            let mut answers = answers.to_vec();
            answers.reverse();
            Self {
                answers,
                asked: Vec::new(),
                said: Vec::new(),
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
