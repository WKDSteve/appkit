//! `CliEngine` against the fake engine of an app that is not the tasks hub: the seam of `appkit-hub`
//! knows no call, no kind, no variable and no script by name, so any engine works the same.
//!
//! The dialect is the one of `rules.nvim`: `rules.*` kinds, `RULES_VAULT`, its list under `rules`.
//! The binary `fake-engine` of this kit plays it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use appkit_hub::engine::{
    probe, CliEngine, CliEngineConfig, Engine, EngineDialect, EngineError, Method, MethodSpec,
    ProbeProblem,
};
use appkit_hub::env::Env;
use appkit_scratch::Scratch;
use serde_json::{json, Value};

static SPECS: [MethodSpec; 3] = [
    MethodSpec::bare("hello", "rules.hello"),
    MethodSpec::bare("snapshot", "rules.snapshot"),
    MethodSpec::read("list", "rules.list"),
];
const HELLO: Method = Method::new(&SPECS[0]);
const SNAPSHOT: Method = Method::new(&SPECS[1]);
const LIST: Method = Method::new(&SPECS[2]);

const DIALECT: EngineDialect = EngineDialect {
    schema: 1,
    schema_tag: None,
    error_kind: "rules.error",
    host_script: "rules-host.lua",
    vault_var: "RULES_VAULT",
    extra_areas_var: "RULES_EXTRA_AREAS",
    required_capabilities: &["hello", "snapshot"],
    hello: HELLO,
    engine_name: "the rules.nvim checkout",
};

/// The engine, with `vault` as its vault: a folder of the test's own (the fake engine writes nothing
/// there, but it is no folder of anybody else either).
fn engine(vault: &Path, env: Env) -> CliEngine {
    engine_of(DIALECT, vault, env)
}

fn engine_of(dialect: EngineDialect, vault: &Path, env: Env) -> CliEngine {
    CliEngine::new(CliEngineConfig {
        dialect,
        nvim: PathBuf::from(env!("CARGO_BIN_EXE_fake-engine")),
        script: vault.join("scripts").join("rules.lua"),
        vault: vault.to_path_buf(),
        lib_dir: None,
        extra_areas: vec!["ALL".to_owned()],
        env,
        timeout: Duration::from_secs(10),
        max_queue: 8,
        max_reply_bytes: 64 * 1024,
    })
}

fn fake(value: Value) -> Value {
    json!({ "fake": value })
}

#[tokio::test]
async fn the_engine_of_the_second_app_is_probed_by_its_own_hello() {
    let vault = Scratch::new("second-dialect");
    let info = probe(&engine(&vault, Env::from_process()), &DIALECT)
        .await
        .expect("probe");
    assert_eq!(info.schemas, vec![1]);
    // Exactly what this engine says it can do; nothing of the tasks engine.
    assert_eq!(info.capabilities, ["hello", "snapshot", "list"]);
}

/// The probe holds the engine against what this dialect needs: an engine that cannot do one of
/// those things is too old, and the problem names what is missing.
#[tokio::test]
async fn an_engine_that_lacks_what_the_dialect_needs_is_too_old() {
    let vault = Scratch::new("second-dialect");
    let needs_more = EngineDialect {
        required_capabilities: &["hello", "snapshot", "task"],
        ..DIALECT
    };
    let err = probe(&engine(&vault, Env::from_process()), &needs_more)
        .await
        .expect_err("too old");
    match err {
        ProbeProblem::TooOld { missing, .. } => assert_eq!(missing, ["task"]),
        other => panic!("{other}"),
    }
}

#[tokio::test]
async fn it_answers_with_its_own_plugin_kind_and_list() {
    let vault = Scratch::new("second-dialect");
    let reply = engine(&vault, Env::from_process())
        .call(SNAPSHOT, None)
        .await
        .expect("answer");
    let doc: Value = serde_json::from_str(&reply.text).unwrap();
    assert_eq!(doc["plugin"], "rules.nvim");
    assert_eq!(doc["kind"], "rules.snapshot");
    assert!(doc["rules"].is_array(), "{doc}");
    assert!(doc.get("tasks").is_none(), "{doc}");
}

#[tokio::test]
async fn the_child_gets_the_variables_of_its_own_dialect_and_none_of_the_other() {
    let vault = Scratch::new("second-dialect");
    let env = Env::from_pairs([
        ("PATH", "C:/bin"),
        ("SystemRoot", "C:/Windows"),
        ("TASKS_VAULT", "C:/from/the/environment"),
        ("TASKS_EXTRA_AREAS", "ELSEWHERE"),
    ]);
    let params = fake(json!({ "echo_env": true }));
    let reply = engine(&vault, env)
        .call(LIST, Some(&params))
        .await
        .expect("answer");
    let doc: Value = serde_json::from_str(&reply.text).unwrap();
    let names: Vec<&str> = doc["env"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(names.contains(&"RULES_VAULT"), "{names:?}");
    assert!(names.contains(&"RULES_EXTRA_AREAS"), "{names:?}");
    for other in ["TASKS_VAULT", "TASKS_EXTRA_AREAS"] {
        assert!(
            !names.contains(&other),
            "{other} reached the child: {names:?}"
        );
    }
}

#[tokio::test]
async fn a_refusal_is_the_error_kind_of_the_dialect() {
    let vault = Scratch::new("second-dialect");
    let params = fake(json!({ "error": "invalid_argument" }));
    let err = engine(&vault, Env::from_process())
        .call(LIST, Some(&params))
        .await
        .expect_err("refusal");
    match err {
        EngineError::Refused(r) => {
            assert_eq!(r.code, "invalid_argument");
            assert!(r.text.contains("rules.error"), "{}", r.text);
        }
        other => panic!("{other}"),
    }
}

#[tokio::test]
async fn an_answer_of_the_other_dialect_is_a_protocol_error() {
    let vault = Scratch::new("second-dialect");
    // The kind of the tasks engine, the schema of nobody: neither is what this engine promised.
    for params in [
        fake(json!({ "kind": "tasks.list" })),
        fake(json!({ "kind": "rules.snapshot" })),
        fake(json!({ "schema": 2 })),
    ] {
        let err = engine(&vault, Env::from_process())
            .call(LIST, Some(&params))
            .await
            .expect_err("protocol");
        assert!(matches!(err, EngineError::Protocol(_)), "{err}");
    }
}

/// An engine that names its contract by a text in every head (testing.nvim: `"testing-export/1"`):
/// a dialect with that `schema_tag` takes exactly the text, and refuses the number and any other text.
#[tokio::test]
async fn a_dialect_with_a_schema_tag_takes_the_text_and_refuses_the_number() {
    let vault = Scratch::new("second-dialect");
    let tagged = EngineDialect {
        schema_tag: Some("rules-export/1"),
        ..DIALECT
    };
    let ok = fake(json!({ "schema": "rules-export/1" }));
    let reply = engine_of(tagged, &vault, Env::from_process())
        .call(LIST, Some(&ok))
        .await
        .expect("answer");
    assert_eq!(reply.kind, "rules.list");
    for params in [
        fake(json!({ "schema": 1 })),
        fake(json!({ "schema": "rules-export/2" })),
        fake(json!({ "schema": "tasks-export/1" })),
    ] {
        let err = engine_of(tagged, &vault, Env::from_process())
            .call(LIST, Some(&params))
            .await
            .expect_err("protocol");
        assert!(matches!(err, EngineError::Protocol(_)), "{params}: {err}");
    }
    // And the dialect without a tag refuses the text, even with the right number in it.
    let err = engine(&vault, Env::from_process())
        .call(LIST, Some(&ok))
        .await
        .expect_err("protocol");
    assert!(matches!(err, EngineError::Protocol(_)), "{err}");
}
