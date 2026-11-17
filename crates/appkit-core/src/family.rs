//! Which apps of the family are on this machine: installed, running, or neither.
//!
//! Two kinds of small files answer it, both read without a network and without a secret:
//!
//! * **Installed**: `<family>/installed/<app>.json`, a manifest that the installer of an app writes
//!   (`appkit-hub install`, the setup of ai-desktop) and its uninstaller removes. `<family>` is
//!   [`family_dir`]: `APPKIT_FAMILY_DIR`, else `%LOCALAPPDATA%\appkit-family` (Windows),
//!   `~/Library/Application Support/appkit-family` (macOS), `$XDG_DATA_HOME/appkit-family` or
//!   `~/.local/share/appkit-family` (elsewhere). A machine-local folder, not the roaming one: what is
//!   installed on one machine is not installed on the next. A manifest whose program file is gone is
//!   a leftover and does not count.
//! * **Running**: `hub.json` in the config folder of the app ([`crate::registry`]), the same file an
//!   app of the kit writes at its start. It counts only while the process it names is alive (and on
//!   Windows only while that process is older than the file, so a reused process id is not taken for
//!   the app).
//!
//! The answer costs a few file reads (microseconds): read it at the start, and again when it may have
//! changed (the window gets the focus, before a hand-over to another app). Port and instance change
//! with every start of the other app.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::registry::{self, HubRecord};

/// The variable that names the family folder (tests, a portable setup).
pub const FAMILY_DIR_VAR: &str = "APPKIT_FAMILY_DIR";

/// What `schema` says in a manifest of this version.
pub const MANIFEST_SCHEMA: &str = "appkit-installed/1";

/// The longest manifest that is read.
const MAX_MANIFEST_BYTES: u64 = 4 * 1024;

/// How many manifests are looked at, at most: the family is a handful of apps.
const MAX_MANIFESTS: usize = 64;

/// A lookup of an environment variable as a path: `None` when it is not set or empty.
pub type Vars<'a> = &'a dyn Fn(&str) -> Option<PathBuf>;

/// What an installer writes to `<family>/installed/<app>.json`.
///
/// Unknown keys are ignored (a later version may add some); `schema` and `app` must be what this
/// version knows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Always [`MANIFEST_SCHEMA`].
    pub schema: String,
    /// The name of the app, as in its `hub.json` and the name of the file.
    pub app: String,
    /// The installed version, when the installer knows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The installed program. Its absence makes the manifest a leftover.
    pub program: PathBuf,
    /// The prefix of the app's variables (`AI_DESKTOP`): `<PREFIX>_CONFIG_DIR` may move its config
    /// folder, and with it `hub.json`.
    pub env_prefix: String,
}

/// An app the caller asks about although it may have no manifest (a build that runs from its
/// checkout).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Member<'a> {
    pub app: &'a str,
    pub env_prefix: &'a str,
}

/// What is known about one app of the family. No path leaves through `Serialize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FamilyApp {
    pub app: String,
    /// A manifest names a program that is there.
    pub installed: bool,
    /// Its `hub.json` names a process that is alive.
    pub running: bool,
    /// The loopback port, while it runs.
    pub port: Option<u16>,
    /// The version of the running app, else the installed one, if known.
    pub version: Option<String>,
    /// The instance of the running app (changes with every start).
    pub instance: Option<String>,
    /// The installed program (for the program, never for a page).
    #[serde(skip)]
    pub program: Option<PathBuf>,
}

/// The family folder of this machine (see the module).
#[must_use]
pub fn family_dir(vars: Vars<'_>) -> Option<PathBuf> {
    if let Some(dir) = vars(FAMILY_DIR_VAR) {
        return Some(dir);
    }
    let base = if cfg!(windows) {
        vars("LOCALAPPDATA")
    } else if cfg!(target_os = "macos") {
        vars("HOME").map(|h| h.join("Library").join("Application Support"))
    } else {
        vars("XDG_DATA_HOME").or_else(|| vars("HOME").map(|h| h.join(".local").join("share")))
    };
    base.map(|b| b.join("appkit-family"))
}

/// The folder of the manifests in the family folder.
#[must_use]
pub fn installed_dir(family: &Path) -> PathBuf {
    family.join("installed")
}

/// The manifest file of `app`.
#[must_use]
pub fn manifest_path(family: &Path, app: &str) -> PathBuf {
    installed_dir(family).join(format!("{app}.json"))
}

/// The config folder of an app, which holds its `hub.json`: `<PREFIX>_CONFIG_DIR`, else `%APPDATA%`,
/// `~/Library/Application Support` (macOS), `$XDG_CONFIG_HOME` or `~/.config`, each with the app's
/// name below it. The same rule as the config folder of a hub of the kit.
#[must_use]
pub fn config_dir(vars: Vars<'_>, app: &str, env_prefix: &str) -> Option<PathBuf> {
    if let Some(dir) = vars(&format!("{env_prefix}_CONFIG_DIR")) {
        return Some(dir);
    }
    let base = if cfg!(windows) {
        vars("APPDATA")
    } else if cfg!(target_os = "macos") {
        vars("HOME").map(|h| h.join("Library").join("Application Support"))
    } else {
        vars("XDG_CONFIG_HOME").or_else(|| vars("HOME").map(|h| h.join(".config")))
    };
    base.map(|b| b.join(app))
}

/// Is `app` a name that may be a file name of the family folder (letters, digits, `-`, `_`, `.`,
/// not starting with a dot)?
#[must_use]
pub fn valid_app_name(app: &str) -> bool {
    !app.is_empty()
        && app.len() <= 64
        && !app.starts_with('.')
        && app
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// Write the manifest of an installed app (through a temporary file, so a reader never sees half).
///
/// # Errors
///
/// An app name that cannot be a file name, or an I/O error.
pub fn write_manifest(family: &Path, manifest: &Manifest) -> io::Result<()> {
    if !valid_app_name(&manifest.app) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("not an app name: {:?}", manifest.app),
        ));
    }
    let dir = installed_dir(family);
    fs::create_dir_all(&dir)?;
    let text = serde_json::to_string_pretty(manifest).map_err(io::Error::other)?;
    let tmp = dir.join(format!("{}.json.{}.tmp", manifest.app, std::process::id()));
    fs::write(&tmp, format!("{text}\n"))?;
    fs::rename(&tmp, manifest_path(family, &manifest.app)).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

/// Remove the manifest of `app` (and ignore that it is already gone).
pub fn remove_manifest(family: &Path, app: &str) {
    if valid_app_name(app) {
        let _ = fs::remove_file(manifest_path(family, app));
    }
}

/// Read the manifest at `path`. `None`: missing, too large, not a manifest of this version, or one
/// whose `app` is not the name of its file. A byte order mark at the start is allowed: a setup
/// (Inno Setup's `SaveStringsToUTF8File`) writes one.
#[must_use]
pub fn read_manifest(path: &Path) -> Option<Manifest> {
    let file = fs::File::open(path).ok()?;
    let mut text = String::new();
    file.take(MAX_MANIFEST_BYTES + 1)
        .read_to_string(&mut text)
        .ok()?;
    if text.len() as u64 > MAX_MANIFEST_BYTES {
        return None;
    }
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let manifest: Manifest = serde_json::from_str(text).ok()?;
    let stem = path.file_stem()?.to_str()?;
    (manifest.schema == MANIFEST_SCHEMA
        && manifest.app == stem
        && valid_app_name(&manifest.app)
        && valid_app_name(&manifest.env_prefix))
    .then_some(manifest)
}

/// The `hub.json` in `config`, if it is a record of `app` whose process is alive.
#[must_use]
pub fn running_record(config: &Path, app: &str) -> Option<HubRecord> {
    let record = registry::read_record(config).ok().flatten()?;
    if record.app != app || record.pid == 0 || record.port == 0 {
        return None;
    }
    let written = fs::metadata(registry::record_path(config))
        .and_then(|m| m.modified())
        .ok();
    process_alive(record.pid, written).then_some(record)
}

/// The apps of the family on this machine: every installed one (by its manifest), and the members
/// of `also` whether installed or not. Sorted by name; an app that is neither installed nor running
/// is in the list only when `also` names it.
#[must_use]
pub fn family_apps(vars: Vars<'_>, also: &[Member<'_>]) -> Vec<FamilyApp> {
    let mut found: BTreeMap<String, (String, Option<Manifest>)> = BTreeMap::new();
    for m in also {
        if valid_app_name(m.app) {
            found.insert(m.app.to_owned(), (m.env_prefix.to_owned(), None));
        }
    }
    if let Some(family) = family_dir(vars) {
        for manifest in manifests(&installed_dir(&family)) {
            found.insert(
                manifest.app.clone(),
                (manifest.env_prefix.clone(), Some(manifest)),
            );
        }
    }
    found
        .into_iter()
        .map(|(app, (prefix, manifest))| look_at(vars, app, &prefix, manifest))
        .collect()
}

/// The state of one app (see [`family_apps`]).
#[must_use]
pub fn family_app(vars: Vars<'_>, member: Member<'_>) -> FamilyApp {
    family_apps(vars, &[member])
        .into_iter()
        .find(|a| a.app == member.app)
        .unwrap_or_else(|| FamilyApp {
            app: member.app.to_owned(),
            installed: false,
            running: false,
            port: None,
            version: None,
            instance: None,
            program: None,
        })
}

fn manifests(dir: &Path) -> Vec<Manifest> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .take(MAX_MANIFESTS)
        .filter_map(|e| read_manifest(&e.path()))
        .collect()
}

fn look_at(vars: Vars<'_>, app: String, prefix: &str, manifest: Option<Manifest>) -> FamilyApp {
    let program = manifest
        .as_ref()
        .map(|m| m.program.clone())
        .filter(|p| p.is_file());
    let record = config_dir(vars, &app, prefix).and_then(|dir| running_record(&dir, &app));
    let version = record
        .as_ref()
        .map(|r| r.version.clone())
        .or_else(|| manifest.as_ref().and_then(|m| m.version.clone()));
    FamilyApp {
        installed: program.is_some(),
        running: record.is_some(),
        port: record.as_ref().map(|r| r.port),
        instance: record.map(|r| r.instance),
        version,
        program,
        app,
    }
}

/// Is the process `pid` alive, and (where the system says when it started) not younger than
/// `written`, the time its `hub.json` was written? A process of another user counts as not alive:
/// an app of the family runs as the person who asks.
#[must_use]
pub fn process_alive(pid: u32, written: Option<SystemTime>) -> bool {
    imp::alive(pid, written)
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    /// A process that started this much after its file was written is another one with the same id.
    const SLACK: Duration = Duration::from_secs(2);

    /// 100 ns intervals from 1601-01-01 to 1970-01-01.
    const EPOCH_DIFF: u64 = 116_444_736_000_000_000;

    fn to_system_time(ft: FILETIME) -> Option<SystemTime> {
        let ticks = (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime);
        let since = ticks.checked_sub(EPOCH_DIFF)?;
        UNIX_EPOCH.checked_add(Duration::from_nanos(since.checked_mul(100)?))
    }

    pub fn alive(pid: u32, written: Option<SystemTime>) -> bool {
        // SAFETY: a plain call; a null handle is checked before use.
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            return false;
        }
        let mut code = 0u32;
        let zero = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
        // SAFETY: `handle` is valid until CloseHandle below; the out pointers live on this stack.
        let (got_code, got_times) = unsafe {
            (
                GetExitCodeProcess(handle, &mut code),
                GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user),
            )
        };
        // SAFETY: the handle OpenProcess gave us, closed once.
        unsafe { CloseHandle(handle) };
        if got_code == 0 || code != STILL_ACTIVE as u32 {
            return false;
        }
        match (
            written,
            (got_times != 0).then(|| to_system_time(created)).flatten(),
        ) {
            (Some(written), Some(started)) => started <= written + SLACK,
            _ => true,
        }
    }
}

#[cfg(unix)]
#[allow(unsafe_code)]
mod imp {
    use std::time::SystemTime;

    pub fn alive(pid: u32, _written: Option<SystemTime>) -> bool {
        let Ok(pid) = libc::pid_t::try_from(pid) else {
            return false;
        };
        if pid <= 0 {
            return false;
        }
        // SAFETY: signal 0 only asks whether the process exists and may be signalled by us.
        unsafe { libc::kill(pid, 0) == 0 }
    }
}

#[cfg(not(any(windows, unix)))]
mod imp {
    use std::time::SystemTime;

    pub fn alive(_pid: u32, _written: Option<SystemTime>) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use appkit_scratch::Scratch;
    use std::collections::HashMap;

    struct Machine {
        root: Scratch,
        vars: HashMap<String, PathBuf>,
    }

    impl Machine {
        fn new(name: &str) -> Self {
            let root = Scratch::new(&format!("family-{name}"));
            let mut vars = HashMap::new();
            vars.insert(FAMILY_DIR_VAR.to_owned(), root.join("family"));
            vars.insert("AI_DESKTOP_CONFIG_DIR".to_owned(), root.join("ai-config"));
            vars.insert(
                "TASKS_DESKTOP_CONFIG_DIR".to_owned(),
                root.join("tasks-config"),
            );
            Self { root, vars }
        }

        fn vars(&self) -> impl Fn(&str) -> Option<PathBuf> + '_ {
            |name| self.vars.get(name).cloned()
        }

        fn family(&self) -> PathBuf {
            self.root.join("family")
        }

        fn install(&self, app: &str, prefix: &str) -> PathBuf {
            let program = self.root.join(format!("{app}.exe"));
            fs::create_dir_all(&*self.root).unwrap();
            fs::write(&program, b"program").unwrap();
            write_manifest(
                &self.family(),
                &Manifest {
                    schema: MANIFEST_SCHEMA.to_owned(),
                    app: app.to_owned(),
                    version: Some("1.2.3".to_owned()),
                    program: program.clone(),
                    env_prefix: prefix.to_owned(),
                },
            )
            .unwrap();
            program
        }

        fn run(&self, config: &str, app: &str, pid: u32) {
            registry::write_record(
                &self.root.join(config),
                &HubRecord {
                    app: app.to_owned(),
                    port: 8080,
                    pid,
                    version: "2.0.0".to_owned(),
                    instance: "cd".repeat(16),
                },
            )
            .unwrap();
        }
    }

    const AI: Member<'static> = Member {
        app: "ai-desktop",
        env_prefix: "AI_DESKTOP",
    };

    #[test]
    fn nothing_there_is_neither_installed_nor_running() {
        let m = Machine::new("none");
        let vars = m.vars();
        assert!(family_apps(&vars, &[]).is_empty());
        let ai = family_app(&vars, AI);
        assert!(!ai.installed && !ai.running);
        assert_eq!((ai.port, ai.version, ai.instance), (None, None, None));
    }

    #[test]
    fn an_installed_app_that_does_not_run() {
        let m = Machine::new("installed");
        let program = m.install("ai-desktop", "AI_DESKTOP");
        let vars = m.vars();
        let apps = family_apps(&vars, &[]);
        assert_eq!(apps.len(), 1);
        let ai = &apps[0];
        assert_eq!(ai.app, "ai-desktop");
        assert!(ai.installed && !ai.running);
        assert_eq!(ai.version.as_deref(), Some("1.2.3"));
        assert_eq!(ai.port, None);
        assert_eq!(ai.program.as_deref(), Some(program.as_path()));
    }

    #[test]
    fn a_running_app_says_port_version_and_instance() {
        let m = Machine::new("running");
        m.install("ai-desktop", "AI_DESKTOP");
        m.run("ai-config", "ai-desktop", std::process::id());
        let vars = m.vars();
        let ai = family_app(&vars, AI);
        assert!(ai.installed && ai.running);
        assert_eq!(ai.port, Some(8080));
        assert_eq!(ai.version.as_deref(), Some("2.0.0"));
        assert_eq!(ai.instance.as_deref(), Some("cd".repeat(16).as_str()));
    }

    #[test]
    fn a_running_build_without_a_manifest_is_found_when_asked_for() {
        let m = Machine::new("dev");
        m.run("ai-config", "ai-desktop", std::process::id());
        let vars = m.vars();
        assert!(family_apps(&vars, &[]).is_empty());
        let ai = family_app(&vars, AI);
        assert!(!ai.installed && ai.running);
    }

    #[test]
    fn a_stale_record_does_not_count_as_running() {
        let m = Machine::new("stale");
        m.install("ai-desktop", "AI_DESKTOP");
        // A process that has ended: its id is free (or another process's, which started later).
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--list")
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        child.wait().unwrap();
        m.run("ai-config", "ai-desktop", pid);
        let vars = m.vars();
        let ai = family_app(&vars, AI);
        assert!(ai.installed && !ai.running);
        assert_eq!(ai.port, None);
        assert_eq!(ai.version.as_deref(), Some("1.2.3"));
    }

    #[cfg(windows)]
    #[test]
    fn a_process_younger_than_its_record_is_another_one() {
        let long_ago = SystemTime::now() - std::time::Duration::from_secs(3600 * 24 * 365 * 30);
        assert!(!process_alive(std::process::id(), Some(long_ago)));
        assert!(process_alive(std::process::id(), Some(SystemTime::now())));
    }

    #[test]
    fn a_record_of_another_app_or_pid_zero_is_not_this_app() {
        let m = Machine::new("foreign");
        m.run("ai-config", "tasks-desktop", std::process::id());
        let vars = m.vars();
        assert!(!family_app(&vars, AI).running);
        m.run("ai-config", "ai-desktop", 0);
        assert!(!family_app(&vars, AI).running);
    }

    #[test]
    fn a_leftover_manifest_whose_program_is_gone_is_not_installed() {
        let m = Machine::new("leftover");
        let program = m.install("ai-desktop", "AI_DESKTOP");
        fs::remove_file(program).unwrap();
        let vars = m.vars();
        let ai = family_app(&vars, AI);
        assert!(!ai.installed);
        assert_eq!(ai.program, None);
    }

    #[test]
    fn removing_the_manifest_uninstalls() {
        let m = Machine::new("remove");
        m.install("ai-desktop", "AI_DESKTOP");
        remove_manifest(&m.family(), "ai-desktop");
        remove_manifest(&m.family(), "ai-desktop");
        let vars = m.vars();
        assert!(family_apps(&vars, &[]).is_empty());
    }

    #[test]
    fn a_manifest_that_is_not_one_is_ignored() {
        let m = Machine::new("bad");
        let dir = installed_dir(&m.family());
        fs::create_dir_all(&dir).unwrap();
        let program = m.root.join("x.exe");
        fs::write(&program, b"x").unwrap();
        let good = |app: &str, schema: &str| {
            serde_json::json!({"schema": schema, "app": app, "program": program, "env_prefix": "X"})
                .to_string()
        };
        // The wrong schema, a name that is not the file's, a large file, no JSON.
        fs::write(dir.join("a.json"), good("a", "appkit-installed/2")).unwrap();
        fs::write(dir.join("b.json"), good("c", MANIFEST_SCHEMA)).unwrap();
        fs::write(dir.join("d.json"), vec![b' '; 8 * 1024]).unwrap();
        fs::write(dir.join("e.json"), "not json").unwrap();
        // An unknown key is a later version's and is fine.
        let mut later: serde_json::Value =
            serde_json::from_str(&good("f", MANIFEST_SCHEMA)).unwrap();
        later["icon"] = serde_json::json!("f.png");
        // With a byte order mark, as a setup writes it.
        fs::write(dir.join("f.json"), format!("\u{feff}{later}\r\n")).unwrap();
        let vars = m.vars();
        let names: Vec<String> = family_apps(&vars, &[]).into_iter().map(|a| a.app).collect();
        assert_eq!(names, ["f"]);
    }

    #[test]
    fn a_name_that_is_no_file_name_is_refused() {
        let m = Machine::new("names");
        for app in ["", "../x", ".hidden", "a/b", "a\\b"] {
            let manifest = Manifest {
                schema: MANIFEST_SCHEMA.to_owned(),
                app: app.to_owned(),
                version: None,
                program: PathBuf::from("x"),
                env_prefix: "X".to_owned(),
            };
            assert!(write_manifest(&m.family(), &manifest).is_err(), "{app:?}");
        }
    }

    #[test]
    fn the_places_follow_the_variables() {
        let vars = |name: &str| match name {
            "LOCALAPPDATA" | "XDG_DATA_HOME" => Some(PathBuf::from("/data")),
            "HOME" => Some(PathBuf::from("/home/u")),
            "APPDATA" | "XDG_CONFIG_HOME" => Some(PathBuf::from("/config")),
            "X_CONFIG_DIR" => Some(PathBuf::from("/moved")),
            _ => None,
        };
        let family = family_dir(&vars).unwrap();
        let config = config_dir(&vars, "y", "Y").unwrap();
        if cfg!(target_os = "macos") {
            assert_eq!(
                family,
                PathBuf::from("/home/u/Library/Application Support/appkit-family")
            );
        } else {
            assert_eq!(family, PathBuf::from("/data/appkit-family"));
            assert_eq!(config, PathBuf::from("/config/y"));
        }
        assert_eq!(config_dir(&vars, "x", "X"), Some(PathBuf::from("/moved")));
        let set = |name: &str| (name == FAMILY_DIR_VAR).then(|| PathBuf::from("/f"));
        assert_eq!(family_dir(&set), Some(PathBuf::from("/f")));
        assert_eq!(family_dir(&|_: &str| None), None);
    }

    #[test]
    fn the_answer_for_a_page_holds_no_path() {
        let m = Machine::new("page");
        m.install("ai-desktop", "AI_DESKTOP");
        let vars = m.vars();
        let text = serde_json::to_string(&family_apps(&vars, &[])).unwrap();
        assert!(
            !text.contains("program") && !text.contains(".exe"),
            "{text}"
        );
    }

    /// `hub.json` as ai-desktop (C++, not built on the kit) wrote it in a real run
    /// (`tests/fixtures/ai-desktop/hub.json`): read with `deny_unknown_fields`, and byte for byte what the kit writes.
    #[test]
    fn the_record_of_ai_desktop_is_read_as_one_of_the_kit() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ai-desktop");
        let record = registry::read_record(&fixture).unwrap().unwrap();
        assert_eq!(
            record,
            HubRecord {
                app: "ai-desktop".to_owned(),
                port: 18093,
                pid: 46668,
                version: "0.1.0".to_owned(),
                instance: "92aed5f0f926392881bab65c665d479e".to_owned(),
            }
        );
        let ours = Scratch::new("family-ai-fixture");
        registry::write_record(&ours, &record).unwrap();
        // Git may give the fixture CRLF on Windows; the bytes of the run are LF.
        let theirs = fs::read_to_string(registry::record_path(&fixture))
            .unwrap()
            .replace("\r\n", "\n");
        assert_eq!(
            fs::read_to_string(registry::record_path(&ours)).unwrap(),
            theirs
        );
        // Found as running only while that process lives; this one ended long ago.
        let vars = |name: &str| (name == "AI_DESKTOP_CONFIG_DIR").then(|| fixture.clone());
        assert!(!family_app(&vars, AI).running || process_alive(46668, None));
    }
}
