//! Find Neovim, the engine checkout and the vault, and probe them.
//!
//! The chain for each value is: the settings file, then an environment
//! variable, then a search. A value that is set decides: if it is wrong the
//! answer is an error that names it, never a quiet fall back to another
//! place. Every step that fails is kept in the problem, so the setup page
//! can list what was tried. No script of the user's own Neovim
//! configuration is ever loaded: the engine runs with `-u NONE`.

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use appkit_core::proc;

use crate::config::{AppSettings, Env, FileConfig, Resolved, Source};
use crate::engine::child_env;

/// The oldest Neovim the engine is tested against (concept 13.3).
pub const MIN_NVIM: (u32, u32, u32) = (0, 12, 0);

/// What the app calls the things that are looked for: the names of its variables, of the checkout
/// of its engine and the script inside it.
#[derive(Debug, Clone, Copy)]
pub struct DiscoverySpec {
    /// The variable that names the vault.
    pub vault_var: &'static str,
    /// The variable that names the Neovim executable.
    pub nvim_var: &'static str,
    /// The variable that names the checkout of the engine.
    pub engine_dir_var: &'static str,
    /// The name of the folder of that checkout below `REPOS_DIR` (`tasks.nvim`).
    pub engine_name: &'static str,
    /// The script of the engine inside the checkout, as the folders and the file name.
    pub engine_script: &'static [&'static str],
}

/// The values of the app's own settings that the search needs and the kit does not know.
#[derive(Debug, Clone, Copy, Default)]
pub struct AppInputs<'a> {
    /// A checkout of lib.nvim set in the settings file.
    pub lib_dir: Option<&'a Path>,
    /// Areas the engine is told about although they have no marker folder.
    pub extra_areas: &'a [String],
}

/// What the hub needs to start the engine.
#[derive(Debug, Clone)]
pub struct Setup {
    pub nvim: Resolved<PathBuf>,
    pub nvim_version: (u32, u32, u32),
    pub engine_dir: Resolved<PathBuf>,
    /// `None` when no checkout was found by the hub's own search; the
    /// engine still looks in places the hub does not (`.deps`, the plugin
    /// manager's folder), so this is a hint and not yet an error.
    pub lib_dir: Option<Resolved<PathBuf>>,
    /// Why `lib_dir` is `None`, with what was tried.
    pub lib_problem: Option<SetupProblem>,
    pub vault: Resolved<PathBuf>,
    pub extra_areas: Vec<String>,
}

/// What stands between the user and a running hub. Each one is shown on a
/// setup page with the places that were tried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupProblem {
    /// A value of the settings file is not acceptable.
    InvalidSetting(String),
    NoVault {
        tried: Vec<String>,
    },
    VaultNotADirectory(PathBuf),
    NvimNotFound {
        tried: Vec<String>,
    },
    NvimProbeFailed(String),
    NvimTooOld {
        found: (u32, u32, u32),
    },
    EngineNotFound {
        /// The name of the checkout that was looked for (`tasks.nvim`).
        name: &'static str,
        tried: Vec<String>,
    },
    LibNotFound {
        tried: Vec<String>,
    },
}

impl SetupProblem {
    /// A short, fixed word for the problem. It carries no path, so it may
    /// be shown to any client of `hello`.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidSetting(_) => "invalid_setting",
            Self::NoVault { .. } => "no_vault",
            Self::VaultNotADirectory(_) => "vault_not_a_directory",
            Self::NvimNotFound { .. } => "nvim_not_found",
            Self::NvimProbeFailed(_) => "nvim_probe_failed",
            Self::NvimTooOld { .. } => "nvim_too_old",
            Self::EngineNotFound { .. } => "engine_not_found",
            Self::LibNotFound { .. } => "lib_not_found",
        }
    }
}

/// The problems of the start that only a person can fix by setting the app
/// up (no vault, or one that is not a folder), as the `code` of the problem:
/// `hello` reports them as `setup_required`. A missing Neovim or engine is a
/// problem of the machine, not of the first start.
#[must_use]
pub fn setup_reason(problem: Option<&'static str>) -> Option<&'static str> {
    problem.filter(|p| matches!(*p, "no_vault" | "vault_not_a_directory"))
}

impl fmt::Display for SetupProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSetting(why) => write!(f, "invalid setting: {why}"),
            Self::NoVault { tried } => {
                write!(f, "no vault is set (tried: {})", tried.join(", "))
            }
            Self::VaultNotADirectory(p) => write!(f, "the vault is not a folder: {}", p.display()),
            Self::NvimNotFound { tried } => {
                write!(f, "Neovim was not found (tried: {})", tried.join(", "))
            }
            Self::NvimProbeFailed(why) => write!(f, "Neovim does not run: {why}"),
            Self::NvimTooOld { found } => write!(
                f,
                "Neovim {}.{}.{} is older than {}.{}.{}",
                found.0, found.1, found.2, MIN_NVIM.0, MIN_NVIM.1, MIN_NVIM.2
            ),
            Self::EngineNotFound { name, tried } => write!(
                f,
                "the {name} checkout was not found (tried: {})",
                tried.join(", ")
            ),
            Self::LibNotFound { tried } => write!(
                f,
                "the lib.nvim checkout was not found (tried: {})",
                tried.join(", ")
            ),
        }
    }
}

impl std::error::Error for SetupProblem {}

/// The engine entry point inside a checkout of the engine.
#[must_use]
pub fn engine_script(spec: &DiscoverySpec, engine_dir: &Path) -> PathBuf {
    spec.engine_script
        .iter()
        .fold(engine_dir.to_path_buf(), |path, part| path.join(part))
}

fn is_engine_dir(spec: &DiscoverySpec, dir: &Path) -> bool {
    engine_script(spec, dir).is_file()
}

fn is_lib_dir(dir: &Path) -> bool {
    dir.join("lua").join("lib").join("nvim").is_dir()
}

/// Make a path absolute without touching the file system. The engine runs
/// in another working folder, so a relative path would mean something else
/// there.
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The file names a program may have on this platform.
fn program_names(base: &str) -> Vec<String> {
    if cfg!(windows) && Path::new(base).extension().is_none() {
        vec![format!("{base}.exe")]
    } else {
        vec![base.to_owned()]
    }
}

/// Look a program up on `PATH`, without a shell.
///
/// An empty or relative entry of `PATH` means "the current folder" to
/// some systems; a program from there is never run, so a file planted in
/// the folder the hub happens to run in is not picked up.
fn search_path(env: &Env, names: &[String]) -> Option<PathBuf> {
    let path = env.get("PATH")?;
    for dir in std::env::split_paths(path) {
        if !dir.is_absolute() {
            continue;
        }
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Parse `NVIM v0.12.2` out of the first line of `nvim --version`.
#[must_use]
pub fn parse_nvim_version(text: &str) -> Option<(u32, u32, u32)> {
    let line = text.lines().next()?;
    let rest = line.trim().strip_prefix("NVIM v")?;
    let core: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let mut parts = core.split('.').map(str::parse::<u32>);
    let major = parts.next()?.ok()?;
    let minor = parts.next()?.ok()?;
    let patch = parts.next().and_then(Result::ok).unwrap_or(0);
    Some((major, minor, patch))
}

fn resolve_vault<A: AppSettings>(
    spec: &DiscoverySpec,
    file: &FileConfig<A>,
    env: &Env,
) -> Result<Resolved<PathBuf>, SetupProblem> {
    let (value, source) = if let Some(v) = &file.vault {
        (v.clone(), Source::Setting)
    } else if let Some(v) = env.path(spec.vault_var) {
        (v, Source::Env(spec.vault_var))
    } else {
        return Err(SetupProblem::NoVault {
            tried: vec![
                "the settings file (vault)".to_owned(),
                format!("the environment variable {}", spec.vault_var),
            ],
        });
    };
    let value = absolute(&value);
    if !value.is_dir() {
        return Err(SetupProblem::VaultNotADirectory(value));
    }
    Ok(Resolved { value, source })
}

fn resolve_nvim<A: AppSettings>(
    spec: &DiscoverySpec,
    file: &FileConfig<A>,
    env: &Env,
) -> Result<Resolved<PathBuf>, SetupProblem> {
    let explicit = if let Some(v) = &file.nvim {
        Some((v.clone(), Source::Setting))
    } else {
        env.path(spec.nvim_var)
            .map(|v| (v, Source::Env(spec.nvim_var)))
    };
    if let Some((value, source)) = explicit {
        // A bare name ("nvim") is looked up on PATH; a path is taken as it is.
        let bare = value.is_relative() && value.components().count() == 1;
        let path = if bare {
            let name = value.to_string_lossy().into_owned();
            search_path(env, &program_names(&name))
        } else {
            Some(absolute(&value))
        };
        return match path {
            Some(value) if value.is_file() => Ok(Resolved { value, source }),
            _ => Err(SetupProblem::NvimNotFound {
                tried: vec![format!("{} (from {source})", value.display())],
            }),
        };
    }
    match search_path(env, &program_names("nvim")) {
        Some(value) => Ok(Resolved {
            value,
            source: Source::Search("PATH".to_owned()),
        }),
        None => Err(SetupProblem::NvimNotFound {
            tried: vec![
                "the settings file (nvim)".to_owned(),
                format!("the environment variable {}", spec.nvim_var),
                "PATH".to_owned(),
            ],
        }),
    }
}

fn resolve_engine<A: AppSettings>(
    spec: &DiscoverySpec,
    file: &FileConfig<A>,
    env: &Env,
) -> Result<Resolved<PathBuf>, SetupProblem> {
    let name = spec.engine_name;
    let explicit = if let Some(v) = &file.engine_dir {
        Some((v.clone(), Source::Setting))
    } else {
        env.path(spec.engine_dir_var)
            .map(|v| (v, Source::Env(spec.engine_dir_var)))
    };
    if let Some((dir, source)) = explicit {
        let dir = absolute(&dir);
        return if is_engine_dir(spec, &dir) {
            Ok(Resolved { value: dir, source })
        } else {
            Err(SetupProblem::EngineNotFound {
                name,
                tried: vec![format!("{} (from {source})", dir.display())],
            })
        };
    }
    if let Some(repos) = env.path("REPOS_DIR") {
        let dir = absolute(&repos.join(name));
        if is_engine_dir(spec, &dir) {
            return Ok(Resolved {
                value: dir,
                source: Source::Search(format!("REPOS_DIR/{name}")),
            });
        }
        return Err(SetupProblem::EngineNotFound {
            name,
            tried: vec![dir.display().to_string()],
        });
    }
    Err(SetupProblem::EngineNotFound {
        name,
        tried: vec![
            "the settings file (engine_dir)".to_owned(),
            format!("the environment variable {}", spec.engine_dir_var),
            format!("REPOS_DIR/{name}"),
        ],
    })
}

fn resolve_lib(
    spec: &DiscoverySpec,
    lib_setting: Option<&Path>,
    env: &Env,
    engine_dir: &Path,
) -> Result<Resolved<PathBuf>, SetupProblem> {
    // What is set decides, in the order the engine itself uses.
    let explicit = if let Some(v) = lib_setting {
        Some((v.to_path_buf(), Source::Setting))
    } else if let Some(v) = env.path("LIB_NVIM_DIR") {
        Some((v, Source::Env("LIB_NVIM_DIR")))
    } else {
        env.path("LIB_NVIM_PATH")
            .map(|v| (v, Source::Env("LIB_NVIM_PATH")))
    };
    if let Some((dir, source)) = explicit {
        let dir = absolute(&dir);
        return if is_lib_dir(&dir) {
            Ok(Resolved { value: dir, source })
        } else {
            Err(SetupProblem::LibNotFound {
                tried: vec![format!("{} (from {source})", dir.display())],
            })
        };
    }
    let mut candidates: Vec<(PathBuf, Source)> = Vec::new();
    if let Some(r) = env.path("REPOS_DIR") {
        candidates.push((
            absolute(&r.join("lib.nvim")),
            Source::Search("REPOS_DIR/lib.nvim".to_owned()),
        ));
    }
    if let Some(parent) = engine_dir.parent() {
        candidates.push((
            parent.join("lib.nvim"),
            Source::Search(format!("next to the {} checkout", spec.engine_name)),
        ));
    }
    let mut tried = Vec::new();
    for (dir, source) in candidates {
        if is_lib_dir(&dir) {
            return Ok(Resolved { value: dir, source });
        }
        tried.push(dir.display().to_string());
    }
    Err(SetupProblem::LibNotFound { tried })
}

/// Ask Neovim for its version, with the same empty environment as the engine.
async fn probe_nvim(nvim: &Path, env: &Env) -> Result<(u32, u32, u32), SetupProblem> {
    let spec = proc::Spec {
        program: nvim.to_path_buf(),
        args: vec![OsString::from("--version")],
        env: child_env(env, None),
        cwd: None,
        stdin: Vec::new(),
        timeout: Duration::from_secs(10),
        max_stdout: 64 * 1024,
        max_stderr: 64 * 1024,
    };
    let out = proc::run(spec)
        .await
        .map_err(|e| SetupProblem::NvimProbeFailed(e.to_string()))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let version = parse_nvim_version(&text).ok_or_else(|| {
        SetupProblem::NvimProbeFailed("the answer of --version is not recognised".to_owned())
    })?;
    if version < MIN_NVIM {
        return Err(SetupProblem::NvimTooOld { found: version });
    }
    Ok(version)
}

/// Resolve everything the engine needs and probe Neovim.
///
/// # Errors
///
/// The first [`SetupProblem`] in the order settings, vault, Neovim, engine.
pub async fn discover<A: AppSettings>(
    spec: &DiscoverySpec,
    file: &FileConfig<A>,
    inputs: AppInputs<'_>,
    env: &Env,
) -> Result<Setup, SetupProblem> {
    file.validate().map_err(SetupProblem::InvalidSetting)?;
    let vault = resolve_vault(spec, file, env)?;
    let nvim = resolve_nvim(spec, file, env)?;
    let nvim_version = probe_nvim(&nvim.value, env).await?;
    let engine_dir = resolve_engine(spec, file, env)?;
    let (lib_dir, lib_problem) = match resolve_lib(spec, inputs.lib_dir, env, &engine_dir.value) {
        Ok(lib) => (Some(lib), None),
        Err(problem) => (None, Some(problem)),
    };
    Ok(Setup {
        nvim,
        nvim_version,
        engine_dir,
        lib_dir,
        lib_problem,
        vault,
        extra_areas: inputs.extra_areas.to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::NoSettings;
    use appkit_scratch::Scratch;

    const SPEC: DiscoverySpec = DiscoverySpec {
        vault_var: "RULES_VAULT",
        nvim_var: "RULES_NVIM",
        engine_dir_var: "RULES_ENGINE_DIR",
        engine_name: "rules.nvim",
        engine_script: &["scripts", "rules.lua"],
    };

    type Cfg = FileConfig<NoSettings>;

    fn resolve_vault_of(file: &Cfg, env: &Env) -> Result<Resolved<PathBuf>, SetupProblem> {
        resolve_vault(&SPEC, file, env)
    }
    fn resolve_engine_of(file: &Cfg, env: &Env) -> Result<Resolved<PathBuf>, SetupProblem> {
        resolve_engine(&SPEC, file, env)
    }
    fn resolve_nvim_of(file: &Cfg, env: &Env) -> Result<Resolved<PathBuf>, SetupProblem> {
        resolve_nvim(&SPEC, file, env)
    }

    fn temp_dir(name: &str) -> Scratch {
        Scratch::new(&format!("hub-{name}"))
    }

    #[test]
    fn parses_release_and_dev_versions() {
        assert_eq!(
            parse_nvim_version("NVIM v0.12.2\nBuild type"),
            Some((0, 12, 2))
        );
        assert_eq!(
            parse_nvim_version("NVIM v0.13.0-dev-123+gabc\n"),
            Some((0, 13, 0))
        );
        assert_eq!(parse_nvim_version("vim 9.0"), None);
    }

    #[test]
    fn a_missing_vault_lists_what_was_tried() {
        let err = resolve_vault_of(&Cfg::default(), &Env::default()).unwrap_err();
        assert!(matches!(err, SetupProblem::NoVault { tried } if tried.len() == 2));
    }

    #[test]
    fn the_setting_beats_the_environment() {
        let scratch = temp_dir("setting");
        let dir = scratch.to_path_buf();
        let file = Cfg {
            vault: Some(dir.clone()),
            ..Cfg::default()
        };
        let env = Env::from_pairs([("RULES_VAULT", "Z:/other")]);
        let got = resolve_vault_of(&file, &env).unwrap();
        assert_eq!(got.source, Source::Setting);
        assert_eq!(got.value, absolute(&dir));
    }

    #[test]
    fn the_variables_are_those_the_spec_names_and_no_others() {
        let dir = temp_dir("variables");
        let env = Env::from_pairs([("RULES_VAULT", dir.as_os_str().to_owned())]);
        let got = resolve_vault_of(&Cfg::default(), &env).unwrap();
        assert_eq!(got.source, Source::Env("RULES_VAULT"));
        // The variable of another app is not read, and the message names ours.
        let other = Env::from_pairs([("TASKS_VAULT", dir.as_os_str().to_owned())]);
        let err = resolve_vault_of(&Cfg::default(), &other).unwrap_err();
        assert!(err.to_string().contains("RULES_VAULT"), "{err}");
        assert!(!err.to_string().contains("TASKS"), "{err}");

        let missing = resolve_engine_of(&Cfg::default(), &Env::default()).unwrap_err();
        assert_eq!(
            missing.to_string(),
            "the rules.nvim checkout was not found (tried: the settings file (engine_dir), \
             the environment variable RULES_ENGINE_DIR, REPOS_DIR/rules.nvim)"
        );
        let none = resolve_nvim_of(&Cfg::default(), &Env::default());
        if let Err(problem) = none {
            assert!(problem.to_string().contains("RULES_NVIM"), "{problem}");
        }
    }

    #[test]
    fn a_vault_that_is_a_file_is_refused() {
        let real = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let file = Cfg {
            vault: Some(real.clone()),
            ..Cfg::default()
        };
        let err = resolve_vault_of(&file, &Env::default()).unwrap_err();
        assert_eq!(err, SetupProblem::VaultNotADirectory(real));
    }

    #[test]
    fn a_relative_vault_is_made_absolute() {
        let file = Cfg {
            vault: Some(PathBuf::from(".")),
            ..Cfg::default()
        };
        let got = resolve_vault_of(&file, &Env::default()).unwrap();
        assert!(got.value.is_absolute(), "{}", got.value.display());
    }

    #[test]
    fn only_a_missing_or_unusable_vault_asks_for_the_setup() {
        for (problem, reason) in [
            (Some("no_vault"), Some("no_vault")),
            (Some("vault_not_a_directory"), Some("vault_not_a_directory")),
            (Some("nvim_not_found"), None),
            (Some("engine_not_found"), None),
            (None, None),
        ] {
            assert_eq!(setup_reason(problem), reason, "{problem:?}");
        }
    }

    #[test]
    fn relative_and_empty_path_entries_are_never_searched() {
        // `cargo test` runs in the package folder, where `Cargo.toml` is a
        // file. PATH names that folder by a relative entry and by an empty
        // one, so a search of relative entries would find it.
        assert!(Path::new("Cargo.toml").is_file(), "the test needs its cwd");
        let sep = if cfg!(windows) { ";" } else { ":" };
        let env = Env::from_pairs([("PATH", format!(".{sep}{sep}relative/bin"))]);
        let found = search_path(&env, &["Cargo.toml".to_owned()]);
        assert_eq!(found, None);
    }

    #[test]
    fn an_absolute_path_entry_is_searched() {
        let dir = temp_dir("path-absolute");
        let name = program_names("appkit-hub-found")[0].clone();
        std::fs::write(dir.join(&name), b"").unwrap();
        let joined = std::env::join_paths([&dir]).unwrap();
        let env = Env::from_pairs([("PATH", joined)]);
        assert_eq!(
            search_path(&env, std::slice::from_ref(&name)),
            Some(dir.join(&name))
        );
    }

    #[test]
    fn a_wrong_engine_setting_does_not_fall_back_to_repos_dir() {
        let repos = temp_dir("repos-fallback");
        let good = repos.join("rules.nvim").join("scripts");
        std::fs::create_dir_all(&good).unwrap();
        std::fs::write(good.join("rules.lua"), b"").unwrap();
        let file = Cfg {
            engine_dir: Some(repos.join("typo.nvim")),
            ..Cfg::default()
        };
        let env = Env::from_pairs([("REPOS_DIR", repos.as_os_str().to_owned())]);
        let err = resolve_engine_of(&file, &env).unwrap_err();
        assert!(matches!(err, SetupProblem::EngineNotFound { .. }), "{err}");
        // Without the setting the search finds it.
        let got = resolve_engine_of(&Cfg::default(), &env).unwrap();
        assert_eq!(got.value, repos.join("rules.nvim"));
        // The place it was found is told with the name of the checkout of this app.
        assert_eq!(
            got.source,
            Source::Search("REPOS_DIR/rules.nvim".to_owned())
        );
    }

    #[test]
    fn lib_is_looked_for_next_to_the_checkout_of_the_engine_of_this_app() {
        let parent = temp_dir("lib-next-to");
        let engine = parent.join("rules.nvim");
        std::fs::create_dir_all(&engine).unwrap();
        std::fs::create_dir_all(parent.join("lib.nvim").join("lua").join("lib").join("nvim"))
            .unwrap();
        // No REPOS_DIR and no setting: the sibling of the engine is the only place.
        let got = resolve_lib(&SPEC, None, &Env::default(), &engine).unwrap();
        assert_eq!(got.value, parent.join("lib.nvim"));
        assert_eq!(
            got.source,
            Source::Search("next to the rules.nvim checkout".to_owned())
        );
        // A setting decides, and is named as such.
        let set = parent.join("lib.nvim");
        let got = resolve_lib(&SPEC, Some(&set), &Env::default(), &engine).unwrap();
        assert_eq!(got.source, Source::Setting);
    }

    #[test]
    fn a_wrong_nvim_setting_is_an_error_not_a_search() {
        let file = Cfg {
            nvim: Some(PathBuf::from("definitely-not-a-program-xyz")),
            ..Cfg::default()
        };
        let err = resolve_nvim_of(&file, &Env::from_process()).unwrap_err();
        assert!(matches!(err, SetupProblem::NvimNotFound { .. }), "{err}");
    }
}
