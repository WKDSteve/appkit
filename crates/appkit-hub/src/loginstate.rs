//! Whether an app starts with the login of the person: one state, read and switched the same way
//! by the app (its `autostart` and the switch of its page) and by hub-desktop (the switch of its
//! tile).
//!
//! The state lives in two places of the machine and nowhere else; nothing keeps a copy of it:
//!
//! - the login entry of the app itself (Windows: its value in the `Run` key), and
//! - for an app that hub-desktop takes over ([`TAKEN_OVER`]): its id in `start_with_hub` of the
//!   settings of hub-desktop, which counts only while hub-desktop itself starts with the login
//!   (hub-desktop then starts that hub in the background).
//!
//! An app is on when either says so. Switching it on uses the list when hub-desktop starts with the
//! login and the app is one it takes over, and the app's own entry otherwise; switching it off takes
//! it out of both. Because both sides read the same two places each time, a change made in the app
//! is what hub-desktop shows and the other way round.
//!
//! The machine's login entries are reached through [`LoginEntries`]: the programs pass
//! [`SystemLogin`], the tests a fake and a folder of their own for the settings of hub-desktop, so
//! that no test ever touches the login of the machine or the settings of the person.

use std::path::{Path, PathBuf};

use appkit_core::identity::AppIdentity;
use serde_json::{Map, Value};

use crate::background::{self, Autostart, Background};
use crate::env::Env;
use crate::loginitems::LoginApp;

/// The names of hub-desktop, the app with the one icon for all hubs. They are on disk (its config
/// folder) and must be what hub-desktop gives itself (hub-desktop tests that they are).
pub const HUB_DESKTOP: AppIdentity = AppIdentity {
    app: "hub-desktop",
    binary: "hub-desktop",
    env_prefix: "HUB_DESKTOP",
    title: "Hub",
    default_port: 17610,
    route_ns: "hub",
};

/// The apps whose login entry hub-desktop takes over (their id in its catalog, which is their
/// `route_ns`): their entry starts their hub in the background (`<program> start`). pult is not one:
/// its entry starts its window, which has an icon of its own.
pub const TAKEN_OVER: [&str; 3] = ["tasks", "rules", "testing"];

/// The key of the settings of hub-desktop that lists the apps it starts with itself.
pub const LIST_KEY: &str = "start_with_hub";

/// The largest settings file of hub-desktop that is read.
const MAX_FILE_BYTES: u64 = 64 * 1024;

/// Whether hub-desktop takes the login entry of the app over.
#[must_use]
pub fn taken_over(identity: &AppIdentity) -> bool {
    TAKEN_OVER.contains(&identity.route_ns)
}

/// The login entries of the machine, one per app.
pub trait LoginEntries {
    /// Whether the app has an entry of its own; `None`: it cannot be told.
    fn is_on(&self, identity: &AppIdentity) -> Option<bool>;

    /// Write the entry of the app so that it starts `program` (`Some`), or remove it (`None`).
    ///
    /// # Errors
    ///
    /// What the system said.
    fn set(&self, identity: &AppIdentity, program: Option<&Path>) -> Result<(), String>;
}

/// The login entries of this machine, through the kit (Windows: `reg.exe` and the `Run` key; Linux and
/// macOS: the files of [`crate::loginitems`]).
pub struct SystemLogin;

fn other(identity: &AppIdentity) -> Background<'_> {
    Background {
        login: LoginApp {
            identity,
            summary: identity.title,
        },
        extra_files: &[],
        uninstall_script: "",
    }
}

impl LoginEntries for SystemLogin {
    fn is_on(&self, identity: &AppIdentity) -> Option<bool> {
        // The state of an entry does not need the program; `Status` never looks at it.
        background::autostart_for(&other(identity), Autostart::Status, Path::new(""))
            .ok()
            .map(|text| text.starts_with("on"))
    }

    fn set(&self, identity: &AppIdentity, program: Option<&Path>) -> Result<(), String> {
        let what = if program.is_some() {
            Autostart::On
        } else {
            Autostart::Off
        };
        background::autostart_for(&other(identity), what, program.unwrap_or(Path::new("")))
            .map(|_| ())
    }
}

/// The program an app's own login entry names: its installed copy (where `<program> install` puts
/// it), never a copy that runs from a build folder or a download, which goes away while the entry
/// would still be shown as on. For [`set`], by the app's page and by hub-desktop's tile alike.
///
/// # Errors
///
/// There is no install folder, or the app is not installed there.
pub fn installed_program(identity: &AppIdentity, env: &Env) -> Result<PathBuf, String> {
    let program = background::installed_program(&other(identity), env)?;
    if program.is_file() {
        Ok(program)
    } else {
        Err(format!(
            "{} is not installed yet: run `{} install`, then switch it on ({} is not there)",
            identity.title,
            identity.binary,
            program.display()
        ))
    }
}

/// The config folder of hub-desktop (where its `config.json` is), as hub-desktop finds it.
#[must_use]
pub fn hub_desktop_dir(env: &Env) -> Option<PathBuf> {
    crate::config::config_dir(env, &HUB_DESKTOP)
}

/// Where an app's start with the login is, as far as it is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    /// Its own login entry.
    Own,
    /// hub-desktop starts it (`start_with_hub`), and hub-desktop starts with the login.
    HubDesktop,
}

/// Whether the app starts with the login, and how. `None`: it cannot be told (the system has no
/// login entries the kit knows, or `reg.exe` did not run). A settings file of hub-desktop that cannot
/// be read lists nothing: hub-desktop cannot start from it either.
#[must_use]
pub fn state(identity: &AppIdentity, env: &Env, login: &dyn LoginEntries) -> Option<Option<Via>> {
    let own = login.is_on(identity);
    if own == Some(true) {
        return Some(Some(Via::Own));
    }
    if taken_over(identity)
        && login.is_on(&HUB_DESKTOP) == Some(true)
        && hub_desktop_dir(env)
            .and_then(|dir| read_list(&dir).ok())
            .is_some_and(|list| list.iter().any(|id| id == identity.route_ns))
    {
        return Some(Some(Via::HubDesktop));
    }
    own.map(|_| None)
}

/// Whether the app starts with the login; `None`: it cannot be told. See [`state`].
#[must_use]
pub fn is_on(identity: &AppIdentity, env: &Env, login: &dyn LoginEntries) -> Option<bool> {
    state(identity, env, login).map(|via| via.is_some())
}

/// Switch the app's start with the login on or off. `program` is what its own entry starts (the
/// installed copy); it is asked for only when the entry is written, so an app that hub-desktop starts
/// needs none. Returns what to print.
///
/// On: hub-desktop starts with the login and takes the app over → the app goes into `start_with_hub`
/// and its own entry is removed (the list first: a failure then starts a hub twice, which its `start`
/// answers with "already running", never none). Otherwise its own entry is written.
///
/// Off: the app is taken out of `start_with_hub` and its own entry is removed.
///
/// # Errors
///
/// A text for the person: the settings of hub-desktop or the entry could not be written, or there is
/// no program to start.
pub fn set(
    identity: &AppIdentity,
    on: bool,
    program: impl FnOnce() -> Result<PathBuf, String>,
    env: &Env,
    login: &dyn LoginEntries,
) -> Result<String, String> {
    let list_dir = taken_over(identity).then(|| hub_desktop_dir(env)).flatten();
    if on {
        if let Some(dir) = list_dir.filter(|_| login.is_on(&HUB_DESKTOP) == Some(true)) {
            edit_list(&dir, |list| {
                if !list.iter().any(|id| id == identity.route_ns) {
                    list.push(identity.route_ns.to_owned());
                }
            })?;
            if login.is_on(identity) != Some(false) {
                login.set(identity, None).map_err(|e| {
                    format!("hub-desktop starts the hub now, but its own login entry stays: {e}")
                })?;
            }
            return Ok(
                "the hub starts with your login, with hub-desktop (start_with_hub)\n".to_owned(),
            );
        }
        let program = program()?;
        login.set(identity, Some(&program))?;
        return Ok(format!(
            "the hub starts with your login: {}\n",
            background::run_command(&program)
        ));
    }
    if let Some(dir) = list_dir
        && read_list(&dir)?.iter().any(|id| id == identity.route_ns)
    {
        edit_list(&dir, |list| list.retain(|id| id != identity.route_ns))?;
    }
    login.set(identity, None)?;
    Ok("the hub does not start with your login\n".to_owned())
}

/// The ids in `start_with_hub` of the settings of hub-desktop in `dir`. A missing file or key is an
/// empty list.
///
/// # Errors
///
/// The file cannot be read, is larger than a settings file or its JSON is not an object whose list
/// holds names.
pub fn read_list(dir: &Path) -> Result<Vec<String>, String> {
    let doc = read_doc(dir)?;
    list_of(&doc)
}

fn list_of(doc: &Map<String, Value>) -> Result<Vec<String>, String> {
    match doc.get(LIST_KEY) {
        None => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str().map(str::to_owned).ok_or_else(|| {
                    format!("{LIST_KEY} of hub-desktop holds a value that is not a name")
                })
            })
            .collect(),
        Some(_) => Err(format!("{LIST_KEY} of hub-desktop is not a list")),
    }
}

fn read_doc(dir: &Path) -> Result<Map<String, Value>, String> {
    use std::io::Read;

    let path = dir.join("config.json");
    let file = match std::fs::File::open(&path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(e) => return Err(format!("cannot read {}: {e}", path.display())),
    };
    let mut text = String::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(format!("{} is larger than a settings file", path.display()));
    }
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(doc)) => Ok(doc),
        Ok(_) => Err(format!("{} is not a JSON object", path.display())),
        Err(e) => Err(format!("{} is not valid: {e}", path.display())),
    }
}

/// Change `start_with_hub` in the settings of hub-desktop in `dir` and nothing else of the file
/// (whatever keys hub-desktop has are kept as they are). Written in one step: a temporary file, then a
/// rename. An empty list removes the key.
fn edit_list(dir: &Path, change: impl FnOnce(&mut Vec<String>)) -> Result<(), String> {
    let mut doc = read_doc(dir)?;
    let mut list = list_of(&doc)?;
    change(&mut list);
    if list.is_empty() {
        doc.remove(LIST_KEY);
    } else {
        doc.insert(
            LIST_KEY.to_owned(),
            Value::Array(list.into_iter().map(Value::String).collect()),
        );
    }
    let text = serde_json::to_string_pretty(&Value::Object(doc)).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("cannot make the config folder of hub-desktop: {e}"))?;
    let tmp = dir.join(format!("config.json.{}.login.tmp", std::process::id()));
    std::fs::write(&tmp, format!("{text}\n"))
        .map_err(|e| format!("cannot write the settings of hub-desktop: {e}"))?;
    std::fs::rename(&tmp, dir.join("config.json")).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("cannot replace the settings of hub-desktop: {e}")
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    use appkit_scratch::Scratch;

    use super::*;

    const TESTING: AppIdentity = AppIdentity {
        app: "testing-desktop",
        binary: "testing-hub",
        env_prefix: "TESTING_DESKTOP",
        title: "Testing",
        default_port: 17617,
        route_ns: "testing",
    };

    const PULT: AppIdentity = AppIdentity {
        app: "pult",
        binary: "pult",
        env_prefix: "PULT",
        title: "pult",
        default_port: 17619,
        route_ns: "pult",
    };

    /// The login entries of a machine made for the test: app → the program its entry starts.
    #[derive(Default)]
    struct FakeLogin {
        entries: RefCell<BTreeMap<&'static str, PathBuf>>,
        unknown: bool,
    }

    impl FakeLogin {
        fn with(apps: &[&'static str]) -> Self {
            let fake = Self::default();
            for app in apps {
                fake.entries
                    .borrow_mut()
                    .insert(app, PathBuf::from(format!("C:/bin/{app}.exe")));
            }
            fake
        }
        fn has(&self, app: &str) -> bool {
            self.entries.borrow().contains_key(app)
        }
    }

    impl LoginEntries for FakeLogin {
        fn is_on(&self, identity: &AppIdentity) -> Option<bool> {
            (!self.unknown).then(|| self.has(identity.app))
        }
        fn set(&self, identity: &AppIdentity, program: Option<&Path>) -> Result<(), String> {
            let mut entries = self.entries.borrow_mut();
            let app: &'static str = Box::leak(identity.app.to_owned().into_boxed_str());
            match program {
                Some(p) => entries.insert(app, p.to_path_buf()),
                None => entries.remove(identity.app),
            };
            Ok(())
        }
    }

    /// A folder of its own for the settings of hub-desktop, named by the variable hub-desktop reads.
    fn profile(name: &str) -> (Scratch, Env) {
        let scratch = Scratch::new(name);
        let env = Env::from_pairs([(
            "HUB_DESKTOP_CONFIG_DIR",
            scratch.path().as_os_str().to_owned(),
        )]);
        (scratch, env)
    }

    fn installed() -> Result<PathBuf, String> {
        Ok(PathBuf::from("C:/apps/testing-hub.exe"))
    }

    fn no_program() -> Result<PathBuf, String> {
        Err("not installed".to_owned())
    }

    #[test]
    fn the_names_are_those_of_hub_desktop() {
        assert_eq!(HUB_DESKTOP.var("CONFIG_DIR"), "HUB_DESKTOP_CONFIG_DIR");
        assert!(taken_over(&TESTING));
        assert!(!taken_over(&PULT));
        assert!(!taken_over(&HUB_DESKTOP));
    }

    #[test]
    fn without_hub_desktop_the_own_entry_is_the_state() {
        let (_s, env) = profile("login-own");
        let login = FakeLogin::default();
        assert_eq!(is_on(&TESTING, &env, &login), Some(false));
        let said = set(&TESTING, true, installed, &env, &login).unwrap();
        assert!(said.contains("testing-hub.exe"), "{said}");
        assert_eq!(state(&TESTING, &env, &login), Some(Some(Via::Own)));
        assert!(
            read_list(env.path("HUB_DESKTOP_CONFIG_DIR").as_deref().unwrap())
                .unwrap()
                .is_empty()
        );
        set(&TESTING, false, no_program, &env, &login).unwrap();
        assert_eq!(is_on(&TESTING, &env, &login), Some(false));
        assert!(!login.has("testing-desktop"));
    }

    #[test]
    fn with_hub_desktop_at_login_the_list_is_switched_and_the_own_entry_goes() {
        let (scratch, env) = profile("login-list");
        std::fs::write(
            scratch.path().join("config.json"),
            r#"{"web_root":"C:/w","tiles":{"tasks":{"mode":"web"}},"start_with_hub":["rules"]}"#,
        )
        .unwrap();
        let login = FakeLogin::with(&["hub-desktop", "testing-desktop"]);
        // On with the own entry already there: it moves into the list; no program is needed.
        set(&TESTING, true, no_program, &env, &login).unwrap();
        assert_eq!(read_list(scratch.path()).unwrap(), ["rules", "testing"]);
        assert!(!login.has("testing-desktop"));
        assert_eq!(state(&TESTING, &env, &login), Some(Some(Via::HubDesktop)));
        // The rest of the file is kept as it was.
        let text = std::fs::read_to_string(scratch.path().join("config.json")).unwrap();
        assert!(text.contains("\"web_root\": \"C:/w\""), "{text}");
        assert!(text.contains("\"mode\": \"web\""), "{text}");
        // Twice on lists it once.
        set(&TESTING, true, no_program, &env, &login).unwrap();
        assert_eq!(read_list(scratch.path()).unwrap(), ["rules", "testing"]);
        // Off takes it out of the list and leaves the other apps.
        set(&TESTING, false, no_program, &env, &login).unwrap();
        assert_eq!(read_list(scratch.path()).unwrap(), ["rules"]);
        assert_eq!(is_on(&TESTING, &env, &login), Some(false));
    }

    #[test]
    fn the_list_counts_only_while_hub_desktop_starts_with_the_login() {
        let (scratch, env) = profile("login-list-off");
        std::fs::write(
            scratch.path().join("config.json"),
            r#"{"start_with_hub":["testing"]}"#,
        )
        .unwrap();
        let login = FakeLogin::default();
        assert_eq!(is_on(&TESTING, &env, &login), Some(false));
        // On writes the own entry then (hub-desktop does not start); the list is left as it is.
        set(&TESTING, true, installed, &env, &login).unwrap();
        assert_eq!(state(&TESTING, &env, &login), Some(Some(Via::Own)));
        assert_eq!(read_list(scratch.path()).unwrap(), ["testing"]);
        // Off clears both, so that hub-desktop at login does not bring it back.
        set(&TESTING, false, no_program, &env, &login).unwrap();
        assert!(read_list(scratch.path()).unwrap().is_empty());
        let text = std::fs::read_to_string(scratch.path().join("config.json")).unwrap();
        assert!(!text.contains(LIST_KEY), "{text}");
    }

    #[test]
    fn an_app_that_is_not_taken_over_keeps_its_own_entry() {
        let (scratch, env) = profile("login-pult");
        let login = FakeLogin::with(&["hub-desktop"]);
        set(
            &PULT,
            true,
            || Ok(PathBuf::from("C:/apps/pult.exe")),
            &env,
            &login,
        )
        .unwrap();
        assert!(login.has("pult"));
        assert!(read_list(scratch.path()).unwrap().is_empty());
        assert_eq!(state(&PULT, &env, &login), Some(Some(Via::Own)));
    }

    #[test]
    fn no_program_and_no_list_is_an_error_that_changes_nothing() {
        let (_s, env) = profile("login-noprog");
        let login = FakeLogin::default();
        assert_eq!(
            set(&TESTING, true, no_program, &env, &login).unwrap_err(),
            "not installed"
        );
        assert!(!login.has("testing-desktop"));
    }

    #[test]
    fn a_broken_settings_file_lists_nothing_and_is_not_overwritten() {
        let (scratch, env) = profile("login-broken");
        std::fs::write(scratch.path().join("config.json"), "{ not json").unwrap();
        let login = FakeLogin::with(&["hub-desktop"]);
        assert_eq!(is_on(&TESTING, &env, &login), Some(false));
        assert!(set(&TESTING, true, no_program, &env, &login).is_err());
        assert_eq!(
            std::fs::read_to_string(scratch.path().join("config.json")).unwrap(),
            "{ not json"
        );
        std::fs::write(
            scratch.path().join("config.json"),
            r#"{"start_with_hub":"testing"}"#,
        )
        .unwrap();
        assert!(read_list(scratch.path()).is_err());
    }

    #[test]
    fn the_own_entry_names_the_installed_copy_or_nothing() {
        let scratch = Scratch::new("login-installed");
        let env = Env::from_pairs([(
            "TESTING_DESKTOP_INSTALL_DIR",
            scratch.path().as_os_str().to_owned(),
        )]);
        let why = installed_program(&TESTING, &env).unwrap_err();
        assert!(why.contains("testing-hub install"), "{why}");
        let copy = scratch.path().join(TESTING.program_file());
        std::fs::write(&copy, b"").unwrap();
        assert_eq!(installed_program(&TESTING, &env).unwrap(), copy);
    }

    #[test]
    fn a_login_that_cannot_be_asked_is_not_told() {
        let (_s, env) = profile("login-unknown");
        let login = FakeLogin {
            unknown: true,
            ..FakeLogin::default()
        };
        assert_eq!(is_on(&TESTING, &env, &login), None);
    }

    /// `<program> autostart` of the app goes through the one state: what it switches is what
    /// hub-desktop reads, and what hub-desktop switched is what it says.
    #[test]
    fn the_command_of_the_app_switches_and_tells_the_one_state() {
        let (scratch, env) = profile("login-command");
        let app = other(&TESTING);
        let login = FakeLogin::with(&["hub-desktop"]);
        let say = |what| background::autostart_with(&app, what, installed, &env, &login).unwrap();
        assert_eq!(say(Autostart::Status), "off\n");
        // Switched on in the app: hub-desktop lists it.
        say(Autostart::On);
        assert_eq!(read_list(scratch.path()).unwrap(), ["testing"]);
        assert_eq!(
            say(Autostart::Status),
            "on: hub-desktop starts it with your login (start_with_hub)\n"
        );
        // Switched off in hub-desktop (the list as its tile writes it): the app says off.
        set(&TESTING, false, no_program, &env, &login).unwrap();
        assert_eq!(say(Autostart::Status), "off\n");
        // Without hub-desktop at login the app's own entry is written and told.
        login.set(&HUB_DESKTOP, None).unwrap();
        say(Autostart::On);
        assert_eq!(say(Autostart::Status), "on: its own login entry\n");
        assert!(login.has("testing-desktop"));
        say(Autostart::Off);
        assert!(!login.has("testing-desktop"));
    }
}
