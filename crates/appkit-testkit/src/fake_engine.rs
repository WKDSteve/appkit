//! A stand-in for `nvim --headless -u NONE -l scripts/<app>.lua call <method>`, and for the
//! persistent host `nvim -l scripts/<app>-host.lua`.
//!
//! An app makes a small binary of it: [`run`] with its dialects (see [`crate::dialect`]) and the
//! documents of its engine ([`Documents`]). The engine it plays is read from the script it is
//! given; the machine here is the same for every app.
//!
//! It answers like the engine would, and does what the request under `"fake"` asks it to, so a
//! test can make it slow, noisy, wrong or loud:
//!
//! * `sleep_ms`: wait before answering;
//! * `stamp`: put `started_ms` and `ended_ms` (since the epoch) in the answer;
//! * `echo_env`: put the names of its environment in the answer;
//! * `echo_stdin`: put the bytes it received on stdin in the answer;
//! * `error`: answer with an error document (the error kind of the dialect) of that code;
//! * `kind`, `schema`: answer with another kind or schema than asked for;
//! * `marker`: write this file after the wait, so a test can tell that the process ran to the end
//!   (or that it never started);
//! * `exit`: print nothing but a line on stderr and end with this status;
//! * `huge`: answer with a string of this many bytes;
//! * `host_exit`, `host_hang` (the host only): crash, or never answer.
//!
//! In the folder of the vault, `.fake-host-starts` counts how often a host was started,
//! `.fake-host-broken` makes it fail before it is ready, and `.fake-host-pid` names the last one.

use std::io::{BufRead, Read, Write};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::dialect::FakeDialect;

/// One request, as the documents of an app see it.
pub struct Call<'a> {
    pub dialect: &'static FakeDialect,
    pub method: &'a str,
    /// The request; `null` when there is none.
    pub params: &'a Value,
    /// The folder the vault variable of the dialect names (empty when it is not set).
    pub vault: String,
}

/// What an app's engine answers, besides what every answer has.
#[derive(Clone, Copy)]
pub struct Documents {
    /// Runs first, before the fakes of the request: a delay or a failure that a test sets up with
    /// a file in the vault (a route builds its own request, so it cannot carry a `fake`).
    pub before: fn(&Call<'_>),
    /// Puts the document of the call into the answer (not for a refusal).
    pub fill: fn(&Call<'_>, &mut Value),
}

fn nothing_before(_: &Call<'_>) {}

/// A snapshot with an empty list, and nothing for any other call.
fn empty_documents(call: &Call<'_>, doc: &mut Value) {
    if call.method == "snapshot" {
        doc[call.dialect.list_key] = json!([]);
    }
}

/// An engine with no documents of its own: enough to test the seam.
pub const NO_DOCUMENTS: Documents = Documents {
    before: nothing_before,
    fill: empty_documents,
};

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// Be the engine: read the arguments, play the dialect the script names, answer one call or be
/// the persistent host.
pub fn run(dialects: &[&'static FakeDialect], documents: Documents) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dialect = FakeDialect::of_args(dialects, &args);
    if args.iter().any(|a| a == "--version") {
        println!("NVIM v0.12.2");
        return;
    }
    if args.iter().any(|a| a.ends_with(dialect.host_script)) {
        host_loop(dialect, documents);
        return;
    }
    let started = now_ms();
    let Some(call) = args.iter().position(|a| a == "call") else {
        eprintln!("fake engine: no call");
        std::process::exit(2);
    };
    let method = args.get(call + 1).cloned().unwrap_or_default();
    let with_params = args.iter().any(|a| a == "--params=-");

    let mut raw = Vec::new();
    if with_params {
        let _ = std::io::stdin().read_to_end(&mut raw);
    }
    let doc = respond(dialect, documents, &method, &raw, started);
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{doc}");
}

/// The persistent host. It says `ready`, then answers one line per request
/// (`<id>\t<method>\t<params>`) with `<id>\t<document>`, until its stdin ends.
fn host_loop(dialect: &'static FakeDialect, documents: Documents) {
    let vault = std::env::var(dialect.vault_var).unwrap_or_default();
    if let Ok(mut starts) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(Path::new(&vault).join(".fake-host-starts"))
    {
        let _ = writeln!(starts, "start");
    }
    if Path::new(&vault).join(".fake-host-broken").exists() {
        eprintln!("fake host: broken");
        std::process::exit(3);
    }
    let _ = std::fs::write(
        Path::new(&vault).join(".fake-host-pid"),
        std::process::id().to_string(),
    );
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "ready\t1");
    let _ = out.flush();
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let mut parts = line.splitn(3, '\t');
        let (Some(id), Some(method), params) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let raw = params.unwrap_or("").as_bytes();
        let fake = serde_json::from_slice::<Value>(raw)
            .ok()
            .and_then(|v| v.get("fake").cloned())
            .unwrap_or(Value::Null);
        // Only a host crashes or hangs on these (the process way, which the
        // caller falls back to, answers the same request).
        if fake.get("host_exit").is_some() {
            eprintln!("fake host: crashed");
            std::process::exit(5);
        }
        if fake.get("host_hang").is_some() {
            std::thread::sleep(Duration::from_secs(3600));
        }
        let doc = respond(dialect, documents, method, raw, now_ms());
        let _ = writeln!(out, "{id}\t{doc}");
        let _ = out.flush();
    }
}

/// The answer of the fake engine to one request.
fn respond(
    dialect: &'static FakeDialect,
    documents: Documents,
    method: &str,
    raw: &[u8],
    started: u64,
) -> Value {
    let params: Value = if raw.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(raw).unwrap_or(Value::Null)
    };
    let fake = params.get("fake").cloned().unwrap_or(Value::Null);
    let call = Call {
        dialect,
        method,
        params: &params,
        vault: std::env::var(dialect.vault_var).unwrap_or_default(),
    };

    (documents.before)(&call);
    if let Some(ms) = fake.get("sleep_ms").and_then(Value::as_u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }
    if let Some(path) = fake.get("marker").and_then(Value::as_str) {
        let _ = std::fs::write(path, b"ran");
    }
    if let Some(code) = fake.get("exit").and_then(Value::as_i64) {
        eprintln!("fake engine failed in {}", call.vault);
        std::process::exit(i32::try_from(code).unwrap_or(1));
    }

    let kind = fake
        .get("kind")
        .and_then(Value::as_str)
        .map_or_else(|| dialect.kind(method).to_owned(), str::to_owned);
    // As given: a number, or the text of an engine that names its contract so (`schema_tag`).
    let schema = fake.get("schema").cloned().unwrap_or_else(|| json!(1));
    let mut doc = json!({
        "schema": schema,
        "plugin": dialect.plugin,
        "kind": kind,
        "generated_at": "2026-01-01T00:00:00Z",
        "vault": {"id": "vault"},
        "engine": {"version": "0.0.0-fake", "git": "0000000", "nvim": "0.12.2"},
    });
    if let Some(code) = fake.get("error").and_then(Value::as_str) {
        doc["kind"] = json!(dialect.error_kind);
        doc["code"] = json!(code);
        doc["message"] = json!("the fake engine refused");
        doc["retryable"] = json!(false);
    } else {
        if method == "hello" {
            doc["schemas"] = json!([1]);
            // What this engine can do, and not more: a hub that asks for anything else must find
            // it missing.
            doc["capabilities"] = json!(dialect.capabilities);
        }
        (documents.fill)(&call, &mut doc);
    }
    if fake.get("stamp").and_then(Value::as_bool) == Some(true) {
        doc["started_ms"] = json!(started);
        doc["ended_ms"] = json!(now_ms());
    }
    if fake.get("echo_env").and_then(Value::as_bool) == Some(true) {
        let mut names: Vec<String> = std::env::vars().map(|(k, _)| k.to_uppercase()).collect();
        names.sort();
        doc["env"] = json!(names);
    }
    if fake.get("echo_stdin").and_then(Value::as_bool) == Some(true) {
        doc["stdin_bytes"] = json!(raw);
    }
    if let Some(n) = fake.get("huge").and_then(Value::as_u64) {
        doc["pad"] = json!("x".repeat(usize::try_from(n).unwrap_or(0)));
    }

    doc
}
