//! The tests of `tools/test_check_code_ref.py`, ported, against the goldens of `schemas/code-ref/1/examples`.

use super::*;
use serde_json::{json, Value};
use std::path::PathBuf;

fn examples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas/code-ref/1/examples")
}

fn golden(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(examples().join(format!("{name}.json"))).unwrap())
        .unwrap()
}

fn check(doc: &Value) -> Vec<String> {
    problems(doc.to_string().as_bytes())
}

fn refused(doc: &Value, needle: &str) {
    let found = check(doc);
    assert!(
        found.iter().any(|l| l.contains(needle)),
        "{needle:?} not in {found:?} for {doc}"
    );
}

fn accepted(doc: &Value) {
    assert_eq!(check(doc), Vec::<String>::new(), "{doc}");
}

#[test]
fn each_golden_passes() {
    let mut names: Vec<String> = std::fs::read_dir(examples())
        .unwrap()
        .filter_map(|e| {
            e.ok()?
                .path()
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .filter(|_| true)
        })
        .collect();
    names.sort();
    names.dedup();
    assert_eq!(names, ["function", "marking", "minimal", "task-body"]);
    for name in ["marking", "function", "minimal"] {
        let raw = std::fs::read(examples().join(format!("{name}.json"))).unwrap();
        assert_eq!(problems(&raw), Vec::<String>::new(), "{name}");
    }
}

#[test]
fn the_golden_task_body_yields_its_two_blocks_and_both_pass() {
    let body = std::fs::read_to_string(examples().join("task-body.md")).unwrap();
    let found = task_problems(&body);
    assert_eq!(found.iter().map(|(i, _)| *i).collect::<Vec<_>>(), [0, 1]);
    assert!(found.iter().all(|(_, p)| p.is_empty()), "{found:?}");
}

#[test]
fn size_is_measured_before_parsing() {
    assert_eq!(
        problems(&vec![b' '; MAX_BYTES + 1]),
        [format!("larger than {MAX_BYTES} bytes")]
    );
    // At the limit it is parsed (and is not an object here), so the check is on the byte count, not on the text.
    assert_eq!(problems(&vec![b' '; MAX_BYTES]), ["not JSON in UTF-8"]);
}

#[test]
fn not_a_document() {
    for raw in [&b""[..], b"[1]", b"\xff\xfe", b"null", &vec![b'['; 100_000]] {
        assert!(!problems(raw).is_empty(), "{raw:?}");
    }
    assert_eq!(problems(b"[1]"), ["not a JSON object"]);
}

#[test]
fn a_higher_or_foreign_version() {
    for schema in [
        json!("code-ref/2"),
        json!("code-ref/0"),
        json!("wkd-intent/1"),
        Value::Null,
        json!(["code-ref/1"]),
    ] {
        let mut doc = golden("function");
        doc["schema"] = schema;
        refused(&doc, "is not code-ref/1");
    }
    let mut doc = golden("function");
    doc.as_object_mut().unwrap().remove("schema");
    refused(&doc, "is not code-ref/1");
}

#[test]
fn required_fields() {
    for key in ["repo", "file", "lines"] {
        let mut doc = golden("minimal");
        doc.as_object_mut().unwrap().remove(key);
        refused(&doc, key);
    }
}

#[test]
fn repo_is_a_name_not_a_path() {
    let long = "x".repeat(101);
    for text in [
        json!("E:\\repos\\x"),
        json!("/home/x"),
        json!("a/b"),
        json!(".."),
        json!(""),
        json!(".x"),
        json!(long),
        json!(1),
    ] {
        let mut doc = golden("minimal");
        doc["repo"] = text;
        refused(&doc, "repo");
    }
    for text in ["docmap.nvim", "a", "A_b-c.d", &"x".repeat(100)] {
        let mut doc = golden("minimal");
        doc["repo"] = json!(text);
        accepted(&doc);
    }
}

#[test]
fn no_way_out_of_the_repository() {
    let escapes = [
        "/etc/passwd",
        "\\windows",
        "C:\\x",
        "c:/x",
        "\\\\host\\x",
        "~/x",
        "$HOME/x",
        "${HOME}/x",
        "%USERPROFILE%\\x",
        "../x",
        "a/../../x",
        "a\\..\\x",
        "..",
    ];
    for text in escapes {
        let mut doc = golden("minimal");
        doc["file"] = json!(text);
        refused(&doc, "file leaves");
        let mut doc = golden("function");
        doc["docmap"]["node"] = json!(text);
        refused(&doc, "docmap.node leaves");
    }
}

#[test]
fn the_escape_words_in_the_middle_of_a_text() {
    // The reference looks for a word anywhere: after white space, a quote or `=`, and `..` also after a slash.
    for text in [
        "a /etc",
        "x=/etc",
        "'~/x",
        "\"C:\\x",
        "a ../x",
        "a/..",
        "a/../b",
        "x\u{a0}%HOME%",
        "v $HOME",
    ] {
        assert!(leaves_repo(text), "{text:?}");
    }
    for text in [
        "a..b",
        "TESTS/x..y/z.lua",
        "a~/b",
        "a/b/c",
        "a$HOMEX",
        "${HOMEX}",
        "100%",
        "%%",
        "ui-kit/src/a b.css",
        "..a",
        "a/..b",
    ] {
        assert!(!leaves_repo(text), "{text:?}");
    }
}

#[test]
fn a_file_is_one_canonical_form() {
    for text in [
        "a\\b.lua",
        "a/b.lua:5",
        "./a.lua",
        "a//b.lua",
        "a/./b.lua",
        "a/",
        "x\0y",
    ] {
        let mut doc = golden("minimal");
        doc["file"] = json!(text);
        refused(&doc, "file is not");
    }
    for text in [
        "a/b.lua",
        "TESTS/x..y/z.lua",
        "a..b",
        "ui-kit/src/a b.css",
        ".github/workflows/ci.yml",
    ] {
        let mut doc = golden("minimal");
        doc["file"] = json!(text);
        accepted(&doc);
    }
    let mut doc = golden("minimal");
    doc["file"] = json!("a/".repeat(151));
    refused(&doc, "file is longer");
    doc["file"] = json!("");
    refused(&doc, "file is missing");
}

#[test]
fn lines() {
    let bad = [
        json!([5, 4]),
        json!([0, 3]),
        json!([-1, 2]),
        json!([1]),
        json!([1, 2, 3]),
        json!(["1", 2]),
        json!([1.5, 2]),
        json!([true, true]),
        json!("1-2"),
        Value::Null,
        json!([1, 10_000_001]),
        json!([1.0, 2.0]),
    ];
    for lines in bad {
        let mut doc = golden("minimal");
        doc["lines"] = lines;
        refused(&doc, "lines");
    }
    // A number beyond 64 bits is no line either, and must not panic.
    let raw =
        br#"{"schema":"code-ref/1","repo":"a","file":"a","lines":[1,99999999999999999999999]}"#;
    assert!(problems(raw).iter().any(|l| l.contains("lines")));
    for lines in [
        json!([10_000_000, 10_000_000]),
        json!([1, 1]),
        json!([3, 9]),
    ] {
        let mut doc = golden("minimal");
        doc["lines"] = lines;
        accepted(&doc);
    }
}

#[test]
fn sha() {
    for text in [
        json!("abc"),
        json!("0123456789ABCDEF0123456789ABCDEF01234567"),
        json!("g".repeat(40)),
        json!(1),
    ] {
        let mut doc = golden("function");
        doc["sha"] = text;
        refused(&doc, "sha");
    }
    for ok in [Value::Null, json!("a".repeat(40)), json!("b".repeat(64))] {
        let mut doc = golden("function");
        doc["sha"] = ok;
        accepted(&doc);
    }
}

#[test]
fn only_released_classes() {
    for class in [
        json!("private"),
        json!("unclassified"),
        Value::Null,
        json!(1),
    ] {
        let mut doc = golden("function");
        doc["class"] = class;
        refused(&doc, "class");
    }
    let doc = golden("minimal");
    assert!(doc.get("class").is_none());
    accepted(&doc);
    for class in ["public", "internal"] {
        let mut doc = golden("function");
        doc["class"] = json!(class);
        accepted(&doc);
    }
}

#[test]
fn excerpt_is_counted_in_bytes() {
    let mut doc = golden("function");
    doc["excerpt"] = json!("x".repeat(MAX_EXCERPT_BYTES));
    accepted(&doc);
    doc["excerpt"] = json!("x".repeat(MAX_EXCERPT_BYTES + 1));
    refused(&doc, "excerpt is longer");
    // 1366 characters, 4098 bytes: short by characters, too long by bytes.
    doc["excerpt"] = json!("\u{20ac}".repeat(1366));
    refused(&doc, "excerpt is longer");
    doc["excerpt"] = json!("\u{20ac}".repeat(1365));
    accepted(&doc);
    doc["excerpt"] = json!(5);
    refused(&doc, "excerpt is not a string");
}

#[test]
fn excerpt_is_code_not_a_path() {
    let mut doc = golden("function");
    doc["excerpt"] = json!("local p = \"/etc/passwd\" .. \"../x\" -- ~/y");
    accepted(&doc);
}

#[test]
fn lengths_and_types_of_the_small_fields() {
    let cases = [
        ("symbol", None, 200),
        ("docmap", Some("node"), 300),
        ("docmap", Some("fn"), 200),
        ("telemetry", Some("namespace"), 100),
        ("telemetry", Some("key"), 300),
    ];
    for (key, sub, max) in cases {
        let mut doc = golden("function");
        let name = sub.map_or_else(|| key.to_owned(), |s| format!("{key}.{s}"));
        let set = |doc: &mut Value, len: usize| match sub {
            None => doc[key] = json!("x".repeat(len)),
            Some(s) => doc[key][s] = json!("x".repeat(len)),
        };
        set(&mut doc, max + 1);
        refused(&doc, &name);
        set(&mut doc, max);
        accepted(&doc);
    }
    let mut doc = golden("function");
    doc["docmap"] = json!({"fn": "node"});
    refused(&doc, "docmap.node");
    let mut doc = golden("function");
    doc["docmap"] = json!({"node": "a.b"});
    accepted(&doc);
    let mut doc = golden("function");
    doc["telemetry"] = json!({"namespace": "x"});
    refused(&doc, "telemetry.key");
    let mut doc = golden("function");
    doc["telemetry"] = json!("x");
    refused(&doc, "telemetry is not an object");
    let mut doc = golden("function");
    doc["dirty"] = json!("no");
    refused(&doc, "dirty");
    let mut doc = golden("function");
    doc["truncated"] = json!(1);
    refused(&doc, "truncated");
    let mut doc = golden("function");
    doc["lang"] = json!("Lua!");
    refused(&doc, "lang");
    // A symbol of 200 characters is long by bytes only when it is counted in bytes: it is not.
    let mut doc = golden("function");
    doc["symbol"] = json!("\u{20ac}".repeat(200));
    accepted(&doc);
}

#[test]
fn control_characters() {
    for key in ["symbol", "excerpt", "repo"] {
        let mut doc = golden("function");
        doc[key] = json!("evil\u{1b}[31m");
        refused(&doc, "control character");
    }
    let mut doc = golden("function");
    doc["excerpt"] = json!("line one\r\nline two\ttab");
    accepted(&doc);
    doc["later"] = json!("a\u{0}b");
    refused(&doc, "control character");
    // C1 and keys count too.
    let mut doc = golden("function");
    doc["later"] = json!("a\u{85}b");
    refused(&doc, "control character");
    let mut doc = golden("function");
    doc["a\u{7}b"] = json!(1);
    refused(&doc, "control character");
}

#[test]
fn deep_nesting_never_panics() {
    for depth in [100, 127, 128, 500, 900, 990, 1100, 20_000, 200_000] {
        let raw = format!(
            "{{\"schema\":\"code-ref/1\",\"later\":{}{}}}",
            "[".repeat(depth),
            "]".repeat(depth)
        );
        let _ = problems(raw.as_bytes());
    }
    // Just under the parser's depth the document is read and its strings are walked without recursion.
    let raw = format!(
        "{{\"schema\":\"code-ref/1\",\"later\":{}\"x\"{}}}",
        "[".repeat(100),
        "]".repeat(100)
    );
    assert!(problems(raw.as_bytes()).iter().any(|l| l.contains("repo")));
    // Past it the document is refused, as not JSON.
    let raw = format!(
        "{{\"schema\":\"code-ref/1\",\"later\":{}{}}}",
        "[".repeat(5000),
        "]".repeat(5000)
    );
    assert_eq!(problems(raw.as_bytes()), ["not JSON in UTF-8"]);
}

#[test]
fn unknown_fields_are_ignored() {
    let mut doc = golden("function");
    doc["later"] = json!({"x": 1});
    doc["docmap"]["later"] = json!(1);
    accepted(&doc);
}

#[test]
fn only_the_language_code_ref_counts() {
    let text = "```json\n{\"schema\": \"code-ref/1\"}\n```\n```code-ref-x\n{}\n```\n";
    assert!(blocks(text).is_empty());
}

#[test]
fn a_reference_inside_a_longer_fence_is_shown_not_read() {
    assert!(blocks("````md\n```code-ref\n{}\n```\n````\n").is_empty());
    assert!(blocks("```\n```code-ref\n{}\n```\n").is_empty());
}

#[test]
fn frontmatter_is_not_body_and_an_unclosed_block_is_dropped() {
    assert!(blocks("---\nnote: |\n  ```code-ref\n---\nbody\n```code-ref\n{}\n").is_empty());
    // The frontmatter hides a block in it, and the body is read after it.
    assert_eq!(
        blocks("---\nn: |\n```code-ref\nx\n```\n---\n```code-ref\n{}\n```\n"),
        ["{}"]
    );
}

#[test]
fn a_bad_block_is_reported_with_its_number() {
    let text = format!(
        "```code-ref\n{}\n```\n```code-ref\n{{\"schema\": \"code-ref/2\"}}\n```\n",
        golden("minimal")
    );
    let found = task_problems(&text);
    assert!(found[0].1.is_empty());
    assert!(!found[1].1.is_empty());
}

#[test]
fn crlf_and_indented_fences() {
    assert_eq!(
        blocks("  ```code-ref\r\n{\"a\": 1}\r\n  ```\r\n"),
        ["{\"a\": 1}"]
    );
    // Four spaces is code, not a fence.
    assert!(blocks("    ```code-ref\n{}\n```\n").is_empty());
    // A longer fence closes only on as many backticks, a shorter line is content.
    assert_eq!(blocks("````code-ref\n```\n{}\n````\n"), ["```\n{}"]);
    // An info string is trimmed, a backtick in it is no fence.
    assert_eq!(blocks("``` code-ref \n{}\n```\n"), ["{}"]);
    assert!(blocks("```code-ref`\n{}\n```\n").is_empty());
}

#[test]
fn each_block_has_its_own_size_limit() {
    let text = format!("```code-ref\n{}\n```\n", " ".repeat(MAX_BYTES + 1));
    assert_eq!(
        task_problems(&text),
        [(0, vec![format!("larger than {MAX_BYTES} bytes")])]
    );
}
