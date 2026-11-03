//! `CliEngine`: one `nvim --headless -l <script> call <method>`
//! process per question (concept 3.5, stage P2), or, with
//! [`CliEngine::with_host`], one persistent host process (the script next to it)
//! that answers many questions (stage P8) and falls back to a process per
//! question when it cannot.
//!
//! The rules that hold here, each with a test:
//!
//! * exactly one question is in flight; the others wait in a bounded FIFO
//!   (the engine's lock is not re-entrant, and a later persistent host must
//!   be able to share this seam);
//! * every process has a watchdog: past its time it is killed;
//! * the child gets the allow-listed environment and nothing else;
//! * a question runs to its end even when the caller gives up, so a client
//!   that disconnects never kills a write half way; a read that nobody waits
//!   for any more is dropped before it starts;
//! * a write that does not end with a document is reported as
//!   [`EngineError::OutcomeUnknown`], never as a plain failure.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use appkit_core::proc::{self, ProcError};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::{oneshot, Mutex};

use super::host::{Failure, Host, HostSpec};
use super::wire::ascii_json;
use super::{
    child_env, CallFuture, Engine, EngineDialect, EngineError, EngineRefusal, Method, Reply,
};
use crate::env::Env;

/// The longest request the engine reads (`api.MAX_REQUEST_BYTES`). A larger
/// one is refused here, before a process is started for nothing.
pub const MAX_REQUEST_BYTES: usize = 256 * 1024;

/// The engine switches the hub sets from its own settings.
pub struct EngineVars<'a> {
    pub dialect: EngineDialect,
    pub vault: &'a Path,
    pub lib_dir: Option<&'a Path>,
    pub extra_areas: &'a [String],
}

/// What a `CliEngine` needs.
#[derive(Debug, Clone)]
pub struct CliEngineConfig {
    /// The contract of the engine this seam speaks.
    pub dialect: EngineDialect,
    pub nvim: PathBuf,
    /// The script of the engine, absolute (`<engine>/scripts/tasks.lua`).
    pub script: PathBuf,
    pub vault: PathBuf,
    pub lib_dir: Option<PathBuf>,
    pub extra_areas: Vec<String>,
    /// The hub's environment; only the allow-list of [`child_env`] is used.
    pub env: Env,
    /// Watchdog per question.
    pub timeout: Duration,
    /// How many questions may wait behind the one in flight.
    pub max_queue: usize,
    /// The longest answer that is accepted.
    pub max_reply_bytes: usize,
}

/// The head every document starts with.
#[derive(Deserialize)]
struct Head {
    /// A number, or the text of a dialect with a `schema_tag`.
    schema: Option<Value>,
    kind: Option<String>,
    code: Option<String>,
    message: Option<String>,
    retryable: Option<bool>,
}

struct Inner {
    cfg: CliEngineConfig,
    /// The one place of the engine; a fair queue.
    gate: Mutex<()>,
    /// Questions that were accepted and are not finished: the one in flight
    /// and those that wait.
    pending: AtomicUsize,
    /// The persistent process, when one is wanted and the engine has one.
    host: Option<Host>,
}

/// A place in the queue. It is given back when the work is finished, not
/// when the caller leaves.
struct Place(Arc<Inner>);

impl Place {
    fn take(inner: &Arc<Inner>) -> Option<Self> {
        // `max_queue` waiting plus the one in flight.
        if inner.pending.fetch_add(1, Ordering::AcqRel) > inner.cfg.max_queue {
            inner.pending.fetch_sub(1, Ordering::AcqRel);
            return None;
        }
        Some(Self(Arc::clone(inner)))
    }
}

impl Drop for Place {
    fn drop(&mut self) {
        self.0.pending.fetch_sub(1, Ordering::AcqRel);
    }
}

pub struct CliEngine {
    inner: Arc<Inner>,
}

/// How many times the usual time a check may take (it reads every file and asks git).
const CHECK_TIMEOUT_FACTOR: u32 = 10;

/// Characters that are no use in a line of text shown to a person: controls
/// and the invisible ones that can reorder or hide text.
pub fn is_unprintable(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{2069}' | '\u{feff}'
        )
}

impl Inner {
    fn args(&self, method: Method, with_params: bool) -> Vec<OsString> {
        // `-u NONE`: no configuration, no plugins. `-i NONE`: no shada file.
        // `-n`: no swap file. The vault comes through the environment, not
        // argv, so no global option can be moved behind the verb.
        let mut args: Vec<OsString> = ["--headless", "-u", "NONE", "-i", "NONE", "-n", "-l"]
            .iter()
            .map(OsString::from)
            .collect();
        args.push(self.cfg.script.clone().into_os_string());
        args.push(OsString::from("call"));
        args.push(OsString::from(method.as_str()));
        if with_params {
            args.push(OsString::from("--params=-"));
        }
        args
    }

    /// Cut paths and unprintable characters out of text that came from the
    /// engine's error output before it is shown anywhere.
    fn scrub(&self, text: &str) -> String {
        let mut roots: Vec<String> = Vec::new();
        if let Some(engine_dir) = self.cfg.script.parent().and_then(Path::parent) {
            roots.push(engine_dir.display().to_string());
        }
        roots.push(self.cfg.vault.display().to_string());
        if let Some(lib) = &self.cfg.lib_dir {
            roots.push(lib.display().to_string());
        }
        for home in ["USERPROFILE", "HOME"] {
            if let Some(h) = self.cfg.env.get(home) {
                roots.push(h.to_string_lossy().into_owned());
            }
        }
        let mut out = text.to_owned();
        for root in roots.into_iter().filter(|r| r.len() > 3) {
            out = out.replace(&root, "<path>");
            out = out.replace(&root.replace('\\', "/"), "<path>");
        }
        let cleaned: String = out
            .chars()
            .map(|c| if is_unprintable(c) { ' ' } else { c })
            .collect();
        let trimmed = cleaned.trim();
        let mut short: String = trimmed.chars().take(300).collect();
        if trimmed.chars().count() > 300 {
            short.push('…');
        }
        short
    }

    /// Run the process and judge what it left. Holds the place of the
    /// engine while it runs.
    async fn execute(
        &self,
        method: Method,
        stdin: Option<String>,
        writes: bool,
        abandoned: impl Fn() -> bool,
    ) -> Result<Reply, EngineError> {
        // The check reads every task file and may ask git: it takes minutes where a question takes a moment. It runs
        // as a process of its own, outside the place and the persistent host, so that a write, a task and a question
        // do not wait behind it; it only reads.
        let aside = method.is_aside();
        let permit = if aside {
            None
        } else {
            Some(self.gate.lock().await)
        };
        // A read that nobody waits for any more is not worth a process (nor
        // is a dry run: it changes nothing). A write is always carried out:
        // its caller may be gone, its client knows it by `client_op_id`.
        if !writes && abandoned() {
            return Err(EngineError::Process("the caller gave up".to_owned()));
        }

        let vars = EngineVars {
            dialect: self.cfg.dialect,
            vault: &self.cfg.vault,
            lib_dir: self.cfg.lib_dir.as_deref(),
            extra_areas: &self.cfg.extra_areas,
        };
        if let Some(host) = self.host.as_ref().filter(|_| !aside) {
            let params = stdin.as_deref().unwrap_or("");
            match host.ask(method.as_str(), params, self.cfg.timeout).await {
                Ok(text) => {
                    let out = proc::Output {
                        code: Some(0),
                        stdout: text.into_bytes(),
                        stderr: Vec::new(),
                    };
                    return match self.judge(method, out) {
                        Err(e) if writes => Err(e.for_write()),
                        other => other,
                    };
                }
                // Nothing reached the host: the process way is safe for a
                // read and for a write.
                Err(Failure::Unavailable(_)) => {}
                // The host ended with the question: a read is asked again, a
                // write may have been carried out.
                Err(Failure::Died(why)) if !writes => {
                    eprintln!("engine host: {} (asking again)", self.scrub(&why));
                }
                Err(Failure::Died(why)) => {
                    return Err(EngineError::Process(self.scrub(&why)).for_write());
                }
                Err(Failure::Timeout) => {
                    return Err(if writes {
                        EngineError::Timeout.for_write()
                    } else {
                        EngineError::Timeout
                    });
                }
                Err(Failure::TooLarge) => {
                    let e = EngineError::Protocol("the answer is larger than the limit".to_owned());
                    return Err(if writes { e.for_write() } else { e });
                }
            }
        }
        let spec = proc::Spec {
            program: self.cfg.nvim.clone(),
            args: self.args(method, stdin.is_some()),
            env: child_env(&self.cfg.env, Some(&vars)),
            cwd: self
                .cfg
                .script
                .parent()
                .and_then(Path::parent)
                .map(Path::to_path_buf),
            stdin: stdin.map(String::into_bytes).unwrap_or_default(),
            timeout: if aside {
                self.cfg.timeout * CHECK_TIMEOUT_FACTOR
            } else {
                self.cfg.timeout
            },
            max_stdout: self.cfg.max_reply_bytes,
            max_stderr: 64 * 1024,
        };
        let result = proc::run(spec).await;
        drop(permit);

        let out = result.map_err(|e| match e {
            ProcError::Spawn(_) => EngineError::Spawn(self.scrub(&e.to_string())),
            ProcError::Timeout(_) => EngineError::Timeout,
            ProcError::TooLarge { stream, .. } => {
                EngineError::Protocol(format!("{stream} is larger than the limit"))
            }
            ProcError::Io(_) => EngineError::Process(self.scrub(&e.to_string())),
        });
        match out.and_then(|o| self.judge(method, o)) {
            Err(e) if writes => Err(e.for_write()),
            other => other,
        }
    }

    /// Turn what the process left into a reply or an error.
    fn judge(&self, method: Method, out: proc::Output) -> Result<Reply, EngineError> {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let mut text = String::from_utf8(out.stdout)
            .map_err(|_| EngineError::Protocol("the answer is not UTF-8".to_owned()))?;
        // The document is one line; the line ending (CRLF on Windows) goes.
        text.truncate(text.trim_end_matches(['\r', '\n']).len());
        if text.is_empty() {
            return Err(EngineError::Failed {
                code: out.code,
                detail: self.scrub(&stderr),
            });
        }
        let Ok(head) = serde_json::from_str::<Head>(&text) else {
            return Err(EngineError::Failed {
                code: out.code,
                detail: self.scrub(&format!("no document: {stderr}")),
            });
        };
        if !self.cfg.dialect.head_schema_matches(head.schema.as_ref()) {
            return Err(EngineError::Protocol(format!(
                "unsupported schema {}",
                self.scrub(&super::shown_schema(head.schema.as_ref()))
            )));
        }
        let kind = head.kind.unwrap_or_default();
        if kind == self.cfg.dialect.error_kind {
            return Err(EngineError::Refused(EngineRefusal {
                code: head.code.unwrap_or_else(|| "internal".to_owned()),
                message: head.message.unwrap_or_default(),
                retryable: head.retryable.unwrap_or(false),
                text,
            }));
        }
        if kind != method.reply_kind() {
            return Err(EngineError::Protocol(format!(
                "{} answered with kind {kind:?}, expected {:?}",
                method.as_str(),
                method.reply_kind()
            )));
        }
        Ok(Reply { kind, text })
    }
}

impl CliEngine {
    /// One process per question.
    #[must_use]
    pub fn new(cfg: CliEngineConfig) -> Self {
        Self::build(cfg, false, Duration::from_secs(5))
    }

    /// A persistent host process when the engine has `scripts/tasks-host.lua`,
    /// a process per question when it has not or when the host fails.
    #[must_use]
    pub fn with_host(cfg: CliEngineConfig) -> Self {
        Self::build(cfg, true, Duration::from_secs(5))
    }

    /// [`CliEngine::with_host`], with the time between two looks at the
    /// engine's files (a change of them replaces the host).
    #[must_use]
    pub fn with_host_checked(cfg: CliEngineConfig, recheck: Duration) -> Self {
        Self::build(cfg, true, recheck)
    }

    fn build(cfg: CliEngineConfig, want_host: bool, recheck: Duration) -> Self {
        let host = want_host
            .then(|| host_script(&cfg))
            .flatten()
            .map(|script| {
                let vars = EngineVars {
                    dialect: cfg.dialect,
                    vault: &cfg.vault,
                    lib_dir: cfg.lib_dir.as_deref(),
                    extra_areas: &cfg.extra_areas,
                };
                Host::new(HostSpec {
                    program: cfg.nvim.clone(),
                    args: ["--headless", "-u", "NONE", "-i", "NONE", "-n", "-l"]
                        .iter()
                        .map(OsString::from)
                        .chain(std::iter::once(script.clone().into_os_string()))
                        .collect(),
                    env: child_env(&cfg.env, Some(&vars)),
                    cwd: cfg
                        .script
                        .parent()
                        .and_then(Path::parent)
                        .map(Path::to_path_buf),
                    max_reply_bytes: cfg.max_reply_bytes,
                    watch: watched_folders(&cfg),
                    recheck,
                })
            });
        Self {
            inner: Arc::new(Inner {
                cfg,
                gate: Mutex::new(()),
                pending: AtomicUsize::new(0),
                host,
            }),
        }
    }

    /// Whether a persistent host process is running right now.
    pub async fn host_running(&self) -> bool {
        match &self.inner.host {
            Some(host) => host.running().await,
            None => false,
        }
    }

    /// End the persistent host (the next question starts another).
    pub async fn stop_host(&self) {
        if let Some(host) = &self.inner.host {
            host.stop().await;
        }
    }

    /// How many questions are accepted and not finished (in flight plus
    /// waiting). For status pages and for tests.
    #[must_use]
    pub fn queued(&self) -> usize {
        self.inner.pending.load(Ordering::Acquire)
    }

    async fn call_inner(
        &self,
        method: Method,
        params: Option<&Value>,
    ) -> Result<Reply, EngineError> {
        let stdin = match (method.takes_params(), params) {
            (false, Some(_)) => {
                return Err(EngineError::BadRequest(format!(
                    "{} takes no parameters",
                    method.as_str()
                )));
            }
            (_, None) if method.needs_params() => {
                return Err(EngineError::BadRequest(format!(
                    "{} needs a request",
                    method.as_str()
                )));
            }
            (_, Some(v)) if !v.is_object() => {
                return Err(EngineError::BadRequest(
                    "the parameters must be a JSON object".to_owned(),
                ));
            }
            (_, Some(v)) => {
                let text = ascii_json(v).map_err(|e| EngineError::BadRequest(e.to_string()))?;
                if text.len() > MAX_REQUEST_BYTES {
                    return Err(EngineError::BadRequest(format!(
                        "the request is larger than {} KiB",
                        MAX_REQUEST_BYTES / 1024
                    )));
                }
                Some(text)
            }
            (_, None) => None,
        };

        // A dry run is a question, not a write: it is dropped like a read, and a
        // failure of it leaves nothing open.
        let writes = method.writes()
            && params
                .and_then(|p| p.get("dry_run"))
                .and_then(Value::as_bool)
                != Some(true);
        let place = Place::take(&self.inner).ok_or(EngineError::Busy)?;
        // The check only reads and can run for minutes: it runs in the future of its caller, so that a caller that is
        // given up (the vault was changed, the hub stops) takes the process down with it (`kill_on_drop`) instead of
        // leaving it to run on beside a new one.
        if method.is_aside() {
            let _place = place;
            return self.inner.execute(method, stdin, false, || false).await;
        }
        let (tx, rx) = oneshot::channel();
        let inner = Arc::clone(&self.inner);
        // The work runs on its own task: dropping the caller's future does
        // not kill a child that may be writing task files.
        tokio::spawn(async move {
            let _place = place;
            let result = inner
                .execute(method, stdin, writes, || tx.is_closed())
                .await;
            let _ = tx.send(result);
        });
        // The sender is gone without an answer only if the task panicked.
        rx.await.unwrap_or_else(|_| {
            let lost = EngineError::Process("the engine task ended without an answer".to_owned());
            Err(if writes { lost.for_write() } else { lost })
        })
    }
}

/// The folders that hold the engine's Lua code: the engine's own and lib.nvim.
fn watched_folders(cfg: &CliEngineConfig) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(engine) = cfg.script.parent().and_then(Path::parent) {
        out.push(engine.join("lua"));
        out.push(engine.join("scripts"));
    }
    if let Some(lib) = &cfg.lib_dir {
        out.push(lib.join("lua"));
    }
    out
}

/// The persistent host next to the script of the engine, if the engine has it.
fn host_script(cfg: &CliEngineConfig) -> Option<PathBuf> {
    let script = cfg.script.with_file_name(cfg.dialect.host_script);
    script.is_file().then_some(script)
}

impl Engine for CliEngine {
    fn call<'a>(&'a self, method: Method, params: Option<&'a Value>) -> CallFuture<'a> {
        Box::pin(self.call_inner(method, params))
    }
}
