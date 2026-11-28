//! Where the hub gets its settings from, and why it picked a value.
//!
//! Every value that is resolved carries its [`Source`], so a setup page can
//! say "the vault is X because the setting says so" instead of guessing
//! (concept 3.4, "Vault-Findung"). Nothing is read from the personal Neovim
//! configuration, and no setting names an executable value other than the
//! path of Neovim itself, which is set on this machine and never over HTTP
//! (concept 5.5 Nr. 9).
//!
//! The file has the keys every app has (the vaults, the port, Neovim and the engine folder) and
//! the keys of the app, which the app names with an [`AppSettings`]. Both parts are strict: a key
//! that neither knows is refused, so a typo is told instead of ignored.

use std::fmt;
use std::path::{Path, PathBuf};

use appkit_core::identity::AppIdentity;
use serde::de::{DeserializeOwned, Error as _, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::value::RawValue;
use serde_json::Value;

pub use crate::env::Env;

/// Where a resolved value came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// The settings file of this device.
    Setting,
    /// An environment variable of the hub process.
    Env(&'static str),
    /// A search, with what was searched.
    Search(String),
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Setting => f.write_str("the settings file"),
            Self::Env(name) => write!(f, "the environment variable {name}"),
            Self::Search(what) => write!(f, "a search ({what})"),
        }
    }
}

/// A value and the reason for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved<T> {
    pub value: T,
    pub source: Source,
}

/// The keys of the settings file that belong to the app and not to every app.
///
/// An implementation derives `Deserialize` with `deny_unknown_fields`: the keys it does not name
/// are then refused. It is read from what is left of the file after the keys of the kit are taken
/// out, and written next to them.
pub trait AppSettings: Default + Clone + fmt::Debug + Serialize + DeserializeOwned {
    /// Check the values that become part of a path or of a list the engine splits itself.
    ///
    /// # Errors
    ///
    /// The reason, as text, for the first value that is not acceptable.
    fn validate(&self) -> Result<(), String> {
        Ok(())
    }
}

/// An app with no settings of its own.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NoSettings {}

impl AppSettings for NoSettings {}

/// The settings file, `config.json` in the config folder.
#[derive(Debug, Clone, Default, Serialize)]
pub struct FileConfig<A = NoSettings> {
    /// The vault folder (the root that holds the areas). The vault called
    /// [`MAIN_VAULT`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vault: Option<PathBuf>,
    /// More vaults (roots that hold areas), by name. The hub reads one at a
    /// time: the one `active` names.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub vaults: Vec<VaultEntry>,
    /// The name of the vault that is read (`main` or an entry of `vaults`);
    /// none means `main`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<String>,
    /// The Neovim executable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nvim: Option<PathBuf>,
    /// A checkout of the engine (the folder with its script).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine_dir: Option<PathBuf>,
    /// What the app keeps in the same file. It is written between `engine_dir` and `port`, where
    /// the keys of the tasks hub always were, so that a file that is saved keeps its order.
    /// What the app keeps in the same file. It is written between `engine_dir` and `port`, where
    /// the keys of the tasks hub always were, so that a file that is saved keeps its order.
    #[serde(flatten)]
    pub app: A,
    /// The port the hub tries first (it is part of the origin).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// `true`: a hub in the background keeps running when no page is connected. Without it such a
    /// hub ends itself after a while (see `background::left_alone`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keep_running: Option<bool>,
}

/// The keys of the kit. `deny_unknown_fields` cannot be used together with `flatten`, so the
/// file is cut in two by key (see the `Deserialize` of [`FileConfig`]). A key that is neither
/// the kit's nor named by the app ends up in the app's part, and it is refused there only if the
/// app's type has `deny_unknown_fields`: [`AppSettings`] asks for it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Core {
    vault: Option<PathBuf>,
    #[serde(default)]
    vaults: Vec<VaultEntry>,
    active: Option<String>,
    nvim: Option<PathBuf>,
    engine_dir: Option<PathBuf>,
    port: Option<u16>,
    keep_running: Option<bool>,
}

const CORE_KEYS: [&str; 7] = [
    "vault",
    "vaults",
    "active",
    "nvim",
    "engine_dir",
    "port",
    "keep_running",
];

/// The top level of the file as text per key, in file order. A key that is there twice is an
/// error, as it is for a derived struct; a value stays text (JSON), so that what is nested in it
/// is read by the derive of its own type, which refuses a repeated key there as well.
struct Top(Vec<(String, Box<RawValue>)>);

impl<'de> Deserialize<'de> for Top {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        struct Seen;

        impl<'de> Visitor<'de> for Seen {
            type Value = Top;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("the settings, an object")
            }

            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Top, M::Error> {
                let mut entries: Vec<(String, Box<RawValue>)> = Vec::new();
                while let Some(key) = map.next_key::<String>()? {
                    if entries.iter().any(|(k, _)| *k == key) {
                        return Err(M::Error::custom(format!("duplicate field `{key}`")));
                    }
                    entries.push((key, map.next_value()?));
                }
                Ok(Top(entries))
            }
        }

        de.deserialize_map(Seen)
    }
}

/// `{"k":v,...}` from the entries, as text.
fn object_text<'a>(entries: impl Iterator<Item = &'a (String, Box<RawValue>)>) -> String {
    let parts: Vec<String> = entries
        .map(|(k, v)| format!("{}:{}", Value::String(k.clone()), v.get()))
        .collect();
    format!("{{{}}}", parts.join(","))
}

impl<'de, A: AppSettings> Deserialize<'de> for FileConfig<A> {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let Top(entries) = Top::deserialize(de)?;
        let is_core = |k: &str| CORE_KEYS.contains(&k);
        let core_text = object_text(entries.iter().filter(|(k, _)| is_core(k)));
        let rest_text = object_text(entries.iter().filter(|(k, _)| !is_core(k)));
        let core: Core = serde_json::from_str(&core_text).map_err(D::Error::custom)?;
        let app: A = serde_json::from_str(&rest_text).map_err(|e| {
            let text = e.to_string();
            // The derive of the app knows its own keys only: the person who mistyped a key of the
            // kit is told those as well.
            if text.starts_with("unknown field") {
                D::Error::custom(format!(
                    "{text}; the keys of every hub are {}",
                    CORE_KEYS.join(", ")
                ))
            } else {
                D::Error::custom(text)
            }
        })?;
        Ok(Self {
            vault: core.vault,
            vaults: core.vaults,
            active: core.active,
            nvim: core.nvim,
            engine_dir: core.engine_dir,
            port: core.port,
            keep_running: core.keep_running,
            app,
        })
    }
}

/// The name of the vault that `vault` (or the environment variable of the app) names.
pub const MAIN_VAULT: &str = "main";
/// The most vaults in the settings.
pub const MAX_VAULTS: usize = 16;

/// One more vault: a name and the folder that holds its areas.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VaultEntry {
    pub name: String,
    pub path: PathBuf,
}

/// Is this a name for a vault: 1 to 40 letters, digits, dot, dash, underscore,
/// not starting with a dot, and not the name of the main vault.
///
/// # Errors
///
/// What is wrong with the name.
pub fn valid_vault_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty() || name.chars().count() > 40 {
        return Err("must have 1 to 40 characters");
    }
    if name.starts_with('.')
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    {
        return Err(
            "may hold letters, digits, dot, dash and underscore, and does not start with a dot",
        );
    }
    if name == MAIN_VAULT {
        return Err("is the name of the main vault");
    }
    Ok(())
}

/// Whether a path names anything but a plain drive or mount: a network share, a device, a verbatim or an NT form.
/// Windows reads `/` and `\` alike, so the separators are made one before the start is looked at (`/\host\share` is
/// a share too). Every prefix but a drive letter (a share, a device, a verbatim or an NT form) begins with two
/// separators, or with `/??/` once they are one, so this is the whole test.
#[must_use]
pub fn is_remote_path(text: &str) -> bool {
    let flat = text.replace('\\', "/");
    flat.starts_with("//") || flat.starts_with("/??/")
}

impl<A: AppSettings> FileConfig<A> {
    /// Check the values that become part of a path or of a list the engine
    /// splits itself, the vaults and what the app checks of its own keys.
    ///
    /// # Errors
    ///
    /// The reason, as text, for the first value that is not acceptable.
    pub fn validate(&self) -> Result<(), String> {
        if self.port.is_some_and(|p| p < 1024) {
            return Err("port: must be 1024 or higher".to_owned());
        }
        self.app.validate()?;
        if self.vaults.len() > MAX_VAULTS {
            return Err(format!("vaults: at most {MAX_VAULTS}"));
        }
        for (i, v) in self.vaults.iter().enumerate() {
            valid_vault_name(&v.name).map_err(|why| format!("vaults: {:?} {why}", v.name))?;
            if !v.path.is_absolute() {
                return Err(format!("vaults: {:?} needs an absolute path", v.name));
            }
            if self.vaults[..i].iter().any(|o| o.name == v.name) {
                return Err(format!("vaults: the name {:?} is there twice", v.name));
            }
        }
        if let Some(active) = self
            .active
            .as_ref()
            .filter(|a| *a != MAIN_VAULT && !self.vaults.iter().any(|v| &v.name == *a))
        {
            return Err(format!("active: there is no vault {active:?}"));
        }
        Ok(())
    }

    /// The settings as they are for the vault that is read: `vault` is the
    /// folder of the active vault.
    #[must_use]
    pub fn effective(&self) -> Self {
        let mut out = self.clone();
        if let Some(entry) = self
            .active
            .as_ref()
            .and_then(|a| self.vaults.iter().find(|v| &v.name == a))
        {
            out.vault = Some(entry.path.clone());
        }
        out
    }

    /// The name of the vault that is read.
    #[must_use]
    pub fn active_name(&self) -> &str {
        self.active.as_deref().unwrap_or(MAIN_VAULT)
    }
}

/// Why the settings file could not be used.
#[derive(Debug)]
pub enum ConfigError {
    /// The file exists but cannot be read.
    Unreadable(PathBuf, std::io::Error),
    /// The file is not valid for this version.
    Invalid(PathBuf, String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable(p, e) => write!(f, "cannot read {}: {e}", p.display()),
            Self::Invalid(p, e) => write!(f, "invalid settings file {}: {e}", p.display()),
        }
    }
}

impl std::error::Error for ConfigError {}

/// The config folder of this app on this device.
///
/// `<PREFIX>_CONFIG_DIR` overrides it; otherwise `%APPDATA%`,
/// `$XDG_CONFIG_HOME` or `~/.config` (and `~/Library/Application Support`
/// on macOS, which is not a promised platform, concept 13.9).
#[must_use]
pub fn config_dir(env: &Env, identity: &AppIdentity) -> Option<PathBuf> {
    let vars = |name: &str| env.path(name);
    appkit_core::family::config_dir(&vars, identity.app, identity.env_prefix)
}

/// Load `config.json` from `dir`. A missing file is an empty config.
///
/// # Errors
///
/// [`ConfigError`] when the file exists and cannot be read or is not valid.
pub fn load_file<A: AppSettings>(dir: &Path) -> Result<FileConfig<A>, ConfigError> {
    use std::io::Read;

    let path = dir.join("config.json");
    let file = match std::fs::File::open(&path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(FileConfig::default()),
        Err(e) => return Err(ConfigError::Unreadable(path, e)),
    };
    // A settings file is a few lines; a device or a huge file is not one.
    let mut text = String::new();
    if let Err(e) = file.take(MAX_FILE_BYTES + 1).read_to_string(&mut text) {
        return Err(ConfigError::Unreadable(path, e));
    }
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(ConfigError::Invalid(
            path,
            format!("larger than {} KiB", MAX_FILE_BYTES / 1024),
        ));
    }
    let cfg: FileConfig<A> = serde_json::from_str(&text)
        .map_err(|e| ConfigError::Invalid(path.clone(), e.to_string()))?;
    cfg.validate()
        .map_err(|why| ConfigError::Invalid(path, why))?;
    Ok(cfg)
}

/// A temporary file in `dir` for a write of `name` that is then renamed over it: unique per call, not only per
/// process, so two writers of one process (two routes of a hub) never share it.
pub(crate) fn temp_beside(dir: &Path, name: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    dir.join(format!("{name}.{}.{n}.tmp", std::process::id()))
}

/// The largest settings file that is read.
const MAX_FILE_BYTES: u64 = 64 * 1024;

/// How long a writer waits for another one to finish with `config.json` before it gives up.
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// Run `f` while this process holds the lock of `config.json` in `dir` (`config.json.lock` beside it, an advisory
/// lock of the system that every writer of the kit takes). A read, a change and the write in `f` then see no write of
/// another process or thread in between: without it, two programs that change different keys of the same file (an app
/// its `start_with_hub` in the settings of hub-desktop, hub-desktop its own keys) lose each other's change. The lock
/// file stays: removing it would let a third writer lock a new file while the second still holds the old one. A
/// writer that holds the lock longer than [`LOCK_WAIT`] makes this give up rather than hang.
///
/// # Errors
///
/// The folder or the lock file cannot be made, the lock is not free in time, or what `f` returns.
pub fn locked<T>(dir: &Path, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot make the config folder: {e}"))?;
    let path = dir.join("config.json.lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    let deadline = std::time::Instant::now() + LOCK_WAIT;
    loop {
        match file.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(format!(
                    "the settings in {} are being written by another program; try again",
                    dir.display()
                ));
            }
            Err(std::fs::TryLockError::Error(e)) => {
                return Err(format!("cannot lock {}: {e}", path.display()));
            }
        }
    }
    let out = f();
    // Closing the file frees the lock as well; this frees it before the caller goes on.
    let _ = file.unlock();
    out
}

/// Change `config.json` in `dir` under its lock ([`locked`]): load it, let `change` change it, write it in one step.
/// What every writer that loads the file to change part of it uses, so that a change of another program between the
/// load and the write is not lost.
///
/// # Errors
///
/// The file cannot be read or is not valid, `change` refuses, or the write fails.
pub fn update_file<A: AppSettings>(
    dir: &Path,
    change: impl FnOnce(&mut FileConfig<A>) -> Result<(), String>,
) -> Result<(), String> {
    locked(dir, || {
        let mut cfg = load_file::<A>(dir).map_err(|e| e.to_string())?;
        change(&mut cfg)?;
        save_file(dir, &cfg)
    })
}

/// Write `config.json` into `dir` in one step (a temporary file, then a
/// rename): a crash leaves the old file or the new one.
///
/// # Errors
///
/// The reason, as text.
pub fn save_file<A: AppSettings>(dir: &Path, cfg: &FileConfig<A>) -> Result<(), String> {
    cfg.validate()?;
    let text = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot make the config folder: {e}"))?;
    let tmp = temp_beside(dir, "config.json");
    std::fs::write(&tmp, format!("{text}\n"))
        .map_err(|e| format!("cannot write the settings: {e}"))?;
    std::fs::rename(&tmp, dir.join("config.json")).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("cannot replace the settings: {e}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use appkit_scratch::Scratch;

    const IDENTITY: AppIdentity = AppIdentity {
        app: "kit-test",
        binary: "kit-test-hub",
        env_prefix: "KIT_TEST",
        title: "Kit",
        default_port: 17999,
        route_ns: "kit",
    };

    type Cfg = FileConfig<NoSettings>;

    /// An app with one key of its own, which it checks.
    #[derive(Debug, Clone, Default, Deserialize, Serialize)]
    #[serde(deny_unknown_fields)]
    struct Mine {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        names: Vec<String>,
    }

    impl AppSettings for Mine {
        fn validate(&self) -> Result<(), String> {
            match self.names.iter().find(|n| n.contains(',')) {
                Some(bad) => Err(format!("names: {bad:?} holds a comma")),
                None => Ok(()),
            }
        }
    }

    #[test]
    fn env_override_wins_for_the_config_dir_and_the_name_comes_from_the_identity() {
        let env = Env::from_pairs([("KIT_TEST_CONFIG_DIR", "C:/x/cfg"), ("APPDATA", "C:/a")]);
        assert_eq!(config_dir(&env, &IDENTITY), Some(PathBuf::from("C:/x/cfg")));
        let env = Env::from_pairs([
            ("APPDATA", "C:/a"),
            ("XDG_CONFIG_HOME", "/xdg"),
            ("HOME", "/h"),
        ]);
        let got = config_dir(&env, &IDENTITY).unwrap();
        assert!(got.ends_with("kit-test"), "{got:?}");
    }

    #[test]
    fn unknown_keys_in_the_file_are_refused_in_both_parts() {
        assert!(serde_json::from_str::<Cfg>(r#"{"vault":"x","vualt":"y"}"#).is_err());
        assert!(serde_json::from_str::<FileConfig<Mine>>(r#"{"vault":"x","nmaes":[]}"#).is_err());
        let ok = serde_json::from_str::<FileConfig<Mine>>(r#"{"vault":"x","names":["a"]}"#);
        assert_eq!(ok.unwrap().app.names, ["a"]);
    }

    #[test]
    fn the_keys_of_the_app_are_written_next_to_those_of_the_kit() {
        let cfg = FileConfig::<Mine> {
            port: Some(20000),
            app: Mine {
                names: vec!["a".to_owned()],
            },
            ..FileConfig::default()
        };
        let text = serde_json::to_string(&cfg).unwrap();
        // The keys of the app sit between the kit's own and the port, the old order of the file.
        assert_eq!(text, r#"{"names":["a"],"port":20000}"#);
        let full = FileConfig::<Mine> {
            vault: Some(PathBuf::from("V")),
            engine_dir: Some(PathBuf::from("E")),
            ..cfg
        };
        assert_eq!(
            serde_json::to_string(&full).unwrap(),
            r#"{"vault":"V","engine_dir":"E","names":["a"],"port":20000}"#
        );
    }

    #[test]
    fn a_key_that_is_there_twice_is_refused_wherever_it_sits() {
        for (what, text) in [
            ("a key of the kit", r#"{"vault":"A","vault":"B"}"#),
            ("a key of the app", r#"{"names":["a"],"names":["b"]}"#),
            (
                "a key inside a vault",
                r#"{"vaults":[{"name":"n","name":"m","path":"/v"}]}"#,
            ),
        ] {
            let err = serde_json::from_str::<FileConfig<Mine>>(text).unwrap_err();
            assert!(err.to_string().contains("duplicate field"), "{what}: {err}");
        }
        // And a file that has each key once is read.
        assert!(serde_json::from_str::<FileConfig<Mine>>(r#"{"vault":"A","names":[]}"#).is_ok());
    }

    #[test]
    fn a_mistyped_key_of_the_kit_is_told_with_the_keys_of_the_kit() {
        let err = serde_json::from_str::<FileConfig<Mine>>(r#"{"vualt":"x"}"#).unwrap_err();
        let text = err.to_string();
        assert!(text.contains("unknown field `vualt`"), "{text}");
        for key in CORE_KEYS {
            assert!(text.contains(key), "{key} is not in: {text}");
        }
        // A value of the wrong type is not an unknown field and carries no list.
        let wrong = serde_json::from_str::<FileConfig<Mine>>(r#"{"names":5}"#).unwrap_err();
        assert!(!wrong.to_string().contains("keys of every hub"), "{wrong}");
    }

    #[test]
    fn what_the_app_checks_is_checked_on_save_and_on_load() {
        let dir = Scratch::new("hub-app-keys");
        let bad = FileConfig::<Mine> {
            app: Mine {
                names: vec!["a,b".to_owned()],
            },
            ..FileConfig::default()
        };
        assert!(save_file(&dir, &bad).is_err());
        let dir = temp_config("app-bad", br#"{"names":["a,b"]}"#);
        assert!(matches!(
            load_file::<Mine>(&dir),
            Err(ConfigError::Invalid(..))
        ));
    }

    #[test]
    fn a_missing_file_is_an_empty_config() {
        let scratch = Scratch::new("hub-no-config");
        let dir = scratch.join("no-such-config-dir");
        let cfg = load_file::<NoSettings>(&dir).expect("missing file is fine");
        assert!(cfg.vault.is_none() && cfg.vaults.is_empty());
    }

    fn temp_config(name: &str, bytes: &[u8]) -> Scratch {
        let dir = Scratch::new(&format!("hub-{name}"));
        std::fs::write(dir.join("config.json"), bytes).unwrap();
        dir
    }

    fn with_vaults(entries: &[(&str, &str)], active: Option<&str>) -> Cfg {
        FileConfig {
            vaults: entries
                .iter()
                .map(|(n, p)| VaultEntry {
                    name: (*n).to_owned(),
                    path: PathBuf::from(p),
                })
                .collect(),
            active: active.map(str::to_owned),
            ..FileConfig::default()
        }
    }

    fn abs() -> &'static str {
        if cfg!(windows) {
            "C:/v"
        } else {
            "/v"
        }
    }

    #[test]
    fn vaults_have_plain_names_absolute_paths_and_an_active_one_that_exists() {
        assert!(with_vaults(&[("native", abs())], Some("native"))
            .validate()
            .is_ok());
        assert!(with_vaults(&[], Some("main")).validate().is_ok());
        for (entries, active) in [
            (vec![("main", abs())], None),
            (vec![("../x", abs())], None),
            (vec![("a b", abs())], None),
            (vec![(".h", abs())], None),
            (vec![("a", "rel/path")], None),
            (vec![("a", abs()), ("a", abs())], None),
            (vec![("a", abs())], Some("b")),
        ] {
            assert!(
                with_vaults(&entries, active).validate().is_err(),
                "{entries:?} {active:?} was accepted"
            );
        }
    }

    /// The order to stay in the background is a key of the kit: written last, absent unless set.
    #[test]
    fn keep_running_is_a_key_of_every_hub() {
        let on: FileConfig = serde_json::from_str(r#"{"keep_running":true,"port":20000}"#).unwrap();
        assert_eq!(on.keep_running, Some(true));
        assert_eq!(
            serde_json::to_string(&on).unwrap(),
            r#"{"port":20000,"keep_running":true}"#
        );
        let unset: FileConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(unset.keep_running, None);
        assert_eq!(serde_json::to_string(&unset).unwrap(), "{}");
        assert!(serde_json::from_str::<FileConfig>(r#"{"keep_running":"yes"}"#).is_err());
    }

    #[test]
    fn the_port_must_not_be_a_privileged_one() {
        let cfg = Cfg {
            port: Some(80),
            ..FileConfig::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn the_active_vault_decides_which_folder_is_read() {
        let mut cfg = with_vaults(&[("native", abs())], Some("native"));
        cfg.vault = Some(PathBuf::from("/main"));
        assert_eq!(cfg.effective().vault, Some(PathBuf::from(abs())));
        assert_eq!(cfg.active_name(), "native");
        cfg.active = None;
        assert_eq!(cfg.effective().vault, Some(PathBuf::from("/main")));
        assert_eq!(cfg.active_name(), MAIN_VAULT);
    }

    #[test]
    fn saving_writes_what_loading_reads_and_refuses_what_is_not_valid() {
        let scratch = Scratch::new("hub-save");
        // A config folder that is not there yet: the save makes it.
        let dir = scratch.join("config");
        let mut cfg = with_vaults(&[("native", abs())], Some("native"));
        cfg.port = Some(20000);
        save_file(&dir, &cfg).unwrap();
        let back = load_file::<NoSettings>(&dir).unwrap();
        assert_eq!(back.vaults, cfg.vaults);
        assert_eq!(
            (back.active, back.port),
            (Some("native".to_owned()), Some(20000))
        );
        let bad = with_vaults(&[("a b", abs())], None);
        assert!(save_file(&dir, &bad).is_err());
        assert!(
            load_file::<NoSettings>(&dir).is_ok(),
            "a refused save leaves the file as it was"
        );
    }

    #[test]
    fn two_saves_of_one_process_at_the_same_time_both_succeed() {
        // Two routes of one hub may save at once; each needs a temporary file of its own, or one rename finds
        // the file gone and reports a failure although the change arrived.
        let scratch = Scratch::new("hub-save-twice");
        let dir = scratch.join("config");
        let saves = |port: u16| {
            let dir = dir.clone();
            std::thread::spawn(move || {
                let mut cfg = with_vaults(&[("native", abs())], Some("native"));
                cfg.port = Some(port);
                (0..200)
                    .map(|_| save_file(&dir, &cfg))
                    .find(Result::is_err)
                    .unwrap_or(Ok(()))
            })
        };
        let (a, b) = (saves(20000), saves(20001));
        assert_eq!(a.join().unwrap(), Ok(()));
        assert_eq!(b.join().unwrap(), Ok(()));
        let port = load_file::<NoSettings>(&dir).unwrap().port;
        assert!(matches!(port, Some(20000 | 20001)), "{port:?}");
        let left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .filter(|n| n != "config.json")
            .collect();
        assert!(left.is_empty(), "left behind: {left:?}");
    }

    #[test]
    fn a_huge_settings_file_is_refused() {
        let text = format!("{{\"vault\":\"{}\"}}", "a".repeat(70_000));
        let dir = temp_config("huge", text.as_bytes());
        assert!(matches!(
            load_file::<NoSettings>(&dir),
            Err(ConfigError::Invalid(..))
        ));
    }

    /// Writers that each add names of their own at the same time, as an app and hub-desktop do: every name is there
    /// in the end (without the lock a load between another writer's load and its write loses that writer's names).
    #[test]
    fn writers_at_the_same_time_lose_nothing_of_each_other() {
        let scratch = Scratch::new("hub-lock");
        let dir = scratch.path().to_owned();
        let threads: Vec<_> = (0..8)
            .map(|t| {
                let dir = dir.clone();
                std::thread::spawn(move || {
                    for n in 0..12 {
                        update_file::<Mine>(&dir, |cfg| {
                            cfg.app.names.push(format!("w{t}-{n}"));
                            Ok(())
                        })
                        .expect("an update");
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().expect("a writer");
        }
        let names = load_file::<Mine>(&dir).expect("the file").app.names;
        assert_eq!(names.len(), 8 * 12, "{names:?}");
    }

    #[test]
    fn a_change_that_refuses_writes_nothing_and_frees_the_lock() {
        let scratch = Scratch::new("hub-lock-refuse");
        let dir = scratch.path();
        let refused = update_file::<Mine>(dir, |_| Err("no".to_owned()));
        assert_eq!(refused, Err("no".to_owned()));
        assert!(!dir.join("config.json").exists());
        update_file::<Mine>(dir, |cfg| {
            cfg.port = Some(20001);
            Ok(())
        })
        .expect("the lock is free again");
        assert_eq!(load_file::<Mine>(dir).unwrap().port, Some(20001));
    }
}
