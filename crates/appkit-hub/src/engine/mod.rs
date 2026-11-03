//! The engine seam: what a hub may ask of its engine (a Neovim plugin) and what comes back.
//!
//! A [`Method`] is a closed list, never a string from outside (concept 5.5
//! Nr. 8). A [`Reply`] is the document as the engine wrote it; the hub
//! checks its head (schema, kind) and passes the text on untouched.

mod cli;
mod host;
mod method;
mod wire;

use std::ffi::OsString;
use std::fmt;
use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

use crate::env::Env;

pub use cli::is_unprintable;
pub use cli::{CliEngine, CliEngineConfig, EngineVars};
pub use host::Failure as HostFailure;
pub use method::{Method, MethodSpec};
pub use wire::ascii_json;

/// What the seam must know about the engine's contract, and nothing else: the version, the kind
/// of a refusal, the script of the persistent host, the names of the switches the child gets, the
/// capabilities the app needs, and how the engine calls itself in a message.
#[derive(Debug, Clone, Copy)]
pub struct EngineDialect {
    /// The contract version the app speaks.
    pub schema: u64,
    /// How the head of every document names the contract when the engine writes it as a text
    /// (testing.nvim: `"testing-export/1"`) instead of the number [`Self::schema`]. `None`: the head
    /// holds the number. `hello` lists the versions as numbers (`schemas`) either way.
    pub schema_tag: Option<&'static str>,
    /// The `kind` of a document by which the engine refuses or fails (`tasks.error`).
    pub error_kind: &'static str,
    /// The file name of the persistent host, next to the script of the engine (`tasks-host.lua`).
    pub host_script: &'static str,
    /// The switch that names the vault (`TASKS_VAULT`).
    pub vault_var: &'static str,
    /// The switch that names areas to add (`TASKS_EXTRA_AREAS`).
    pub extra_areas_var: &'static str,
    /// What the read side of the app needs from the engine.
    pub required_capabilities: &'static [&'static str],
    /// The call that asks the engine about itself.
    pub hello: Method,
    /// How a message names the engine ("the tasks.nvim checkout").
    pub engine_name: &'static str,
}

impl EngineDialect {
    /// Does the `schema` of a document's head name the contract this app speaks? The number
    /// [`Self::schema`], or, for a dialect with a [`Self::schema_tag`], exactly that text: the other
    /// form is refused, so a document of another contract with the same number does not pass.
    #[must_use]
    pub fn head_schema_matches(&self, schema: Option<&Value>) -> bool {
        match (self.schema_tag, schema) {
            (Some(tag), Some(Value::String(s))) => s == tag,
            (None, Some(n)) => n.as_u64() == Some(self.schema),
            _ => false,
        }
    }
}

/// How much of a foreign `schema` an error message shows.
const SHOWN_SCHEMA_CHARS: usize = 64;

/// The `schema` of a head as a message may show it: its JSON text (control characters escaped), at
/// most [`SHOWN_SCHEMA_CHARS`] characters, `none` when there is none. The engine wrote it, so it is
/// not repeated whole: a text of any length would go into every error message.
pub(crate) fn shown_schema(schema: Option<&Value>) -> String {
    let Some(value) = schema else {
        return "none".to_owned();
    };
    let text = value.to_string();
    if text.chars().count() <= SHOWN_SCHEMA_CHARS {
        return text;
    }
    let mut cut: String = text.chars().take(SHOWN_SCHEMA_CHARS).collect();
    cut.push_str("...");
    cut
}

/// An answer of the engine: one JSON document on one line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub kind: String,
    /// The document text, with the line ending removed.
    pub text: String,
}

/// A `tasks.error` document: the engine refused or failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineRefusal {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    /// The document text, ready to be relayed.
    pub text: String,
}

/// Why a call produced no answer.
#[derive(Debug)]
pub enum EngineError {
    /// The engine answered with a `tasks.error` document.
    Refused(EngineRefusal),
    /// The request itself is not one the hub will send (checked before any
    /// process starts).
    BadRequest(String),
    /// More calls are waiting than the hub keeps; try again.
    Busy,
    /// The engine did not answer within its time and was killed.
    Timeout,
    /// The process could not be started: nothing ran, nothing changed.
    Spawn(String),
    /// The process started but its pipes failed.
    Process(String),
    /// The process ended without a document (exit status and a scrubbed
    /// excerpt of its error output).
    Failed { code: Option<i32>, detail: String },
    /// A document came back that does not fit the contract.
    Protocol(String),
    /// A write (`ops`) started and did not end with a document: some of it
    /// may have been written. The caller must read again before it believes
    /// anything about the vault; the text says what happened.
    OutcomeUnknown(String),
}

impl EngineError {
    /// What a failure means for a write: once the process has started, an
    /// error that is not the engine's own refusal leaves the outcome open.
    #[must_use]
    pub fn for_write(self) -> Self {
        match self {
            Self::Timeout | Self::Process(_) | Self::Failed { .. } | Self::Protocol(_) => {
                Self::OutcomeUnknown(self.to_string())
            }
            other => other,
        }
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(r) => write!(f, "engine refused ({}): {}", r.code, r.message),
            Self::BadRequest(m) => write!(f, "bad request: {m}"),
            Self::Busy => f.write_str("the engine queue is full"),
            Self::Timeout => f.write_str("the engine did not answer in time"),
            Self::Spawn(m) => write!(f, "engine could not be started: {m}"),
            Self::Process(m) => write!(f, "engine process: {m}"),
            Self::Failed { code, detail } => match code {
                Some(c) => write!(f, "engine failed (exit {c}): {detail}"),
                None => write!(f, "engine failed: {detail}"),
            },
            Self::Protocol(m) => write!(f, "engine protocol: {m}"),
            Self::OutcomeUnknown(m) => write!(f, "the outcome of the write is unknown ({m})"),
        }
    }
}

impl std::error::Error for EngineError {}

/// The boxed future of a call, so the trait stays object safe.
pub type CallFuture<'a> = Pin<Box<dyn Future<Output = Result<Reply, EngineError>> + Send + 'a>>;

/// The engine, as the hub sees it. `CliEngine` is the one of P2; a
/// persistent host (P8) implements the same trait.
pub trait Engine: Send + Sync {
    /// Ask one question. `params` is the object of the request: the
    /// parameters of a read method, the whole request of `ops`.
    fn call<'a>(&'a self, method: Method, params: Option<&'a Value>) -> CallFuture<'a>;
}

/// The variables a child process gets, and nothing else (concept 5.5 Nr. 11).
///
/// `PATH`, the system root and the temp and home variables are what any
/// program needs. The three engine switches are set from the hub's own
/// settings, never copied from whatever the environment holds; every other
/// switch of the engine beyond them (trust, marker docs, files) is left out on purpose.
#[must_use]
pub fn child_env(env: &Env, engine: Option<&EngineVars<'_>>) -> Vec<(OsString, OsString)> {
    const PASS: &[&str] = &[
        "PATH",
        "PATHEXT",
        "REPOS_DIR",
        "SystemRoot",
        "windir",
        "TEMP",
        "TMP",
        "TMPDIR",
        "HOME",
        "USERPROFILE",
    ];
    let mut out: Vec<(OsString, OsString)> = PASS
        .iter()
        .filter_map(|name| env.get(name).map(|v| (OsString::from(name), v.clone())))
        .collect();
    if let Some(vars) = engine {
        out.push((OsString::from(vars.dialect.vault_var), vars.vault.into()));
        if let Some(lib) = vars.lib_dir {
            out.push((OsString::from("LIB_NVIM_DIR"), lib.into()));
        }
        if !vars.extra_areas.is_empty() {
            out.push((
                OsString::from(vars.dialect.extra_areas_var),
                vars.extra_areas.join(",").into(),
            ));
        }
    }
    out
}

/// What the engine says about itself, from `hello`.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct EngineInfo {
    pub engine: EngineBuild,
    pub schemas: Vec<u64>,
    pub capabilities: Vec<String>,
}

/// The build of the engine and the Neovim it runs in.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct EngineBuild {
    pub version: String,
    pub git: String,
    pub nvim: String,
}

/// Why an engine cannot be used. Each one is a setup page, not a crash.
#[derive(Debug)]
pub enum ProbeProblem {
    /// The call itself failed.
    Call(EngineError),
    /// The engine does not speak this contract version.
    ContractUnsupported { schemas: Vec<u64>, needs: u64 },
    /// The engine is older than this app: it lacks these capabilities.
    TooOld {
        missing: Vec<String>,
        engine_name: &'static str,
    },
}

impl ProbeProblem {
    /// A short, fixed word for the problem, free of paths (see
    /// `SetupProblem::code`).
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Call(_) => "engine_unreachable",
            Self::ContractUnsupported { .. } => "contract_unsupported",
            Self::TooOld { .. } => "engine_too_old",
        }
    }
}

impl fmt::Display for ProbeProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Call(e) => write!(f, "the engine did not answer hello: {e}"),
            Self::ContractUnsupported { schemas, needs } => write!(
                f,
                "the engine speaks contract versions {schemas:?}, this app needs {needs}"
            ),
            Self::TooOld {
                missing,
                engine_name,
            } => write!(
                f,
                "{engine_name} is too old, it lacks: {}",
                missing.join(", ")
            ),
        }
    }
}

impl std::error::Error for ProbeProblem {}

/// Ask `hello` and check the answer against what this app needs.
///
/// # Errors
///
/// A [`ProbeProblem`] when the engine does not answer, speaks another
/// contract version or lacks a capability.
pub async fn probe(
    engine: &dyn Engine,
    dialect: &EngineDialect,
) -> Result<EngineInfo, ProbeProblem> {
    let reply = engine
        .call(dialect.hello, None)
        .await
        .map_err(ProbeProblem::Call)?;
    let info: EngineInfo = serde_json::from_str(&reply.text).map_err(|e| {
        ProbeProblem::Call(EngineError::Protocol(format!("hello is not readable: {e}")))
    })?;
    if !info.schemas.contains(&dialect.schema) {
        return Err(ProbeProblem::ContractUnsupported {
            schemas: info.schemas,
            needs: dialect.schema,
        });
    }
    let missing: Vec<String> = dialect
        .required_capabilities
        .iter()
        .filter(|c| !info.capabilities.iter().any(|have| have == *c))
        .map(|c| (*c).to_owned())
        .collect();
    if !missing.is_empty() {
        return Err(ProbeProblem::TooOld {
            missing,
            engine_name: dialect.engine_name,
        });
    }
    Ok(info)
}

#[cfg(test)]
mod dialect_tests {
    use super::*;

    static HELLO: MethodSpec = MethodSpec::bare("hello", "rules.hello");

    const RULES: EngineDialect = EngineDialect {
        schema: 2,
        schema_tag: None,
        error_kind: "rules.error",
        host_script: "rules-host.lua",
        vault_var: "RULES_VAULT",
        extra_areas_var: "RULES_EXTRA_AREAS",
        required_capabilities: &["hello"],
        hello: Method::new(&HELLO),
        engine_name: "the rules.nvim checkout",
    };

    #[test]
    fn the_child_gets_the_switches_the_dialect_names_and_no_others() {
        let env = Env::from_pairs([
            ("PATH", "/bin"),
            ("TASKS_VAULT", "/not/this"),
            ("RULES_VAULT", "/not/this/either"),
        ]);
        let vars = EngineVars {
            dialect: RULES,
            vault: std::path::Path::new("/the/vault"),
            lib_dir: None,
            extra_areas: &["a".to_owned(), "b".to_owned()],
        };
        let got: Vec<(String, String)> = child_env(&env, Some(&vars))
            .into_iter()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.to_string_lossy().into_owned(),
                )
            })
            .collect();
        assert!(got.contains(&("RULES_VAULT".to_owned(), "/the/vault".to_owned())));
        assert!(got.contains(&("RULES_EXTRA_AREAS".to_owned(), "a,b".to_owned())));
        // Nothing of the environment's own switches is passed on, whatever the app calls them.
        assert!(!got.iter().any(|(k, _)| k == "TASKS_VAULT"));
        assert!(!got.iter().any(|(_, v)| v.starts_with("/not/this")));
    }

    #[test]
    fn a_problem_names_the_engine_and_the_version_of_the_dialect() {
        let old = ProbeProblem::TooOld {
            missing: vec!["x".to_owned()],
            engine_name: RULES.engine_name,
        };
        assert!(old
            .to_string()
            .starts_with("the rules.nvim checkout is too old"));
        let schema = ProbeProblem::ContractUnsupported {
            schemas: vec![1],
            needs: RULES.schema,
        };
        assert!(schema.to_string().contains("needs 2"), "{schema}");
    }

    #[test]
    fn a_head_names_the_contract_by_the_number_or_by_the_tag_of_the_dialect_never_by_the_other() {
        use serde_json::json;
        // The number of the dialect, and nothing that only looks like it.
        assert!(RULES.head_schema_matches(Some(&json!(2))));
        for bad in [
            json!(1),
            json!(2.5),
            json!("2"),
            json!("rules-export/2"),
            json!(null),
        ] {
            assert!(!RULES.head_schema_matches(Some(&bad)), "{bad}");
        }
        assert!(!RULES.head_schema_matches(None));
        // A dialect with a tag takes exactly that text: not the number, not another contract's tag.
        let tagged = EngineDialect {
            schema: 1,
            schema_tag: Some("testing-export/1"),
            ..RULES
        };
        assert!(tagged.head_schema_matches(Some(&json!("testing-export/1"))));
        for bad in [
            json!(1),
            json!("testing-export/2"),
            json!("tasks-export/1"),
            json!("testing-export/1 "),
            json!(null),
        ] {
            assert!(!tagged.head_schema_matches(Some(&bad)), "{bad}");
        }
        assert!(!tagged.head_schema_matches(None));
    }

    #[test]
    fn a_foreign_schema_is_shown_short_and_escaped() {
        use serde_json::json;
        assert_eq!(shown_schema(None), "none");
        assert_eq!(shown_schema(Some(&json!(2))), "2");
        assert_eq!(shown_schema(Some(&json!("x/1"))), r#""x/1""#);
        let long = shown_schema(Some(&json!("y".repeat(10_000))));
        assert_eq!(long.chars().count(), SHOWN_SCHEMA_CHARS + 3, "{long}");
        assert!(long.ends_with("..."), "{long}");
        // A line break or an escape of the engine stays escaped: one line, nothing for a terminal.
        let raw = shown_schema(Some(&json!("a\nb\u{1b}[2J")));
        assert!(!raw.chars().any(char::is_control), "{raw:?}");
        // Cut at a character, not in the middle of one.
        let wide = shown_schema(Some(&json!("\u{e4}".repeat(200))));
        assert_eq!(wide.chars().count(), SHOWN_SCHEMA_CHARS + 3, "{wide}");
    }
}
