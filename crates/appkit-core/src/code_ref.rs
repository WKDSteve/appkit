//! The reader's check of a `code-ref/1` (`schemas/code-ref/1/README.md`): a Rust port of `tools/check-code-ref.py`.
//!
//! A code-ref is untrusted input, wherever it came from (a task file, a clipboard, a job). [`problems`] never panics on
//! one: it takes the bytes and names every reason to refuse them, and an empty list means the reader may use the
//! reference. [`blocks`] finds the fenced `code-ref` blocks of a task body, [`task_problems`] checks each of them.
//!
//! Same rules, same order and the same words as the reference, with two differences that only make this port
//! stricter: the JSON parser stops at a nesting depth of 128 (the reference refuses deeper documents only when the
//! interpreter gives up), and a lone surrogate escape (`\ud800`) is not JSON here. The text of the schema reason shows
//! the JSON of the value, where the reference shows its Python `repr`. The tests read the goldens of the schema and
//! hold the refusals of `tools/test_check_code_ref.py`.

use serde_json::{Map, Value};

/// The whole document, in UTF-8 bytes, measured before parsing.
pub const MAX_BYTES: usize = 64 * 1024;
/// The excerpt, in UTF-8 bytes (not characters).
pub const MAX_EXCERPT_BYTES: usize = 4096;
/// The only version this code reads.
pub const SCHEMA: &str = "code-ref/1";
/// The language of the fence a task holds a code-ref in.
pub const FENCE_LANG: &str = "code-ref";
/// The longest `file`, in characters.
pub const MAX_PATH: usize = 300;
/// The last line number.
pub const MAX_LINE: i128 = 10_000_000;

const CLASSES: [&str; 2] = ["public", "internal"];

fn limit(name: &str) -> usize {
    match name {
        "symbol" | "docmap.fn" => 200,
        "docmap.node" | "telemetry.key" => 300,
        _ => 100, // telemetry.namespace
    }
}

/// `[A-Za-z0-9][A-Za-z0-9._-]{0,99}`, the whole text.
fn is_repo(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && text.len() <= 100
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// `[a-z0-9][a-z0-9+#._-]{0,31}`, the whole text.
fn is_lang(text: &str) -> bool {
    let mut chars = text.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && text.len() <= 32
        && chars.all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '+' | '#' | '.' | '_' | '-')
        })
}

/// 40 or 64 lowercase hex characters.
fn is_sha(text: &str) -> bool {
    matches!(text.len(), 40 | 64)
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Python's `\s` for text: Unicode white space and the four separators `\x1c`..`\x1f`.
fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn is_slash(c: char) -> bool {
    c == '/' || c == '\\'
}

/// Control characters other than tab, line break and carriage return, and the C1 block.
fn is_control(c: char) -> bool {
    matches!(c, '\u{0}'..='\u{8}' | '\u{b}' | '\u{c}' | '\u{e}'..='\u{1f}' | '\u{7f}'..='\u{9f}')
}

/// An absolute path, a home, a drive or a variable, starting at `at` (`ESCAPE`, first half, after its boundary).
fn starts_absolute(chars: &[char], at: usize) -> bool {
    let Some(&first) = chars.get(at) else {
        return false;
    };
    let at_is = |i: usize, f: fn(char) -> bool| chars.get(i).copied().is_some_and(f);
    if is_slash(first) {
        return true;
    }
    if first.is_ascii_alphabetic() && chars.get(at + 1) == Some(&':') && at_is(at + 2, is_slash) {
        return true;
    }
    if first == '~' && at_is(at + 1, is_slash) {
        return true;
    }
    if first == '$' {
        let rest = if chars.get(at + 1) == Some(&'{') {
            at + 2
        } else {
            at + 1
        };
        // `\$\{?HOME\b`: the braces are optional, and the word must end there.
        for start in [rest, at + 1] {
            let home = ['H', 'O', 'M', 'E'];
            if (0..4).all(|k| chars.get(start + k) == Some(&home[k])) && !at_is(start + 4, is_word)
            {
                return true;
            }
        }
    }
    if first == '%' {
        let letters = chars[at + 1..]
            .iter()
            .take_while(|c| c.is_ascii_alphabetic())
            .count();
        return letters > 0 && chars.get(at + 1 + letters) == Some(&'%');
    }
    false
}

/// A `..` segment, starting at `at` (`ESCAPE`, second half, after its boundary).
fn starts_dotdot(chars: &[char], at: usize) -> bool {
    chars.get(at) == Some(&'.')
        && chars.get(at + 1) == Some(&'.')
        && chars
            .get(at + 2)
            .is_none_or(|&c| is_slash(c) || is_space(c))
}

/// Does `text` hold one word that leaves the repository: an absolute path, a home, a `..` segment?
#[must_use]
pub fn leaves_repo(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    // A match begins at the start of the text, or after one boundary character that it consumes.
    if starts_absolute(&chars, 0) || starts_dotdot(&chars, 0) {
        return true;
    }
    chars.iter().enumerate().any(|(i, &c)| {
        let first_half = is_space(c) || matches!(c, '"' | '\'' | '=');
        let second_half = first_half || is_slash(c);
        (first_half && starts_absolute(&chars, i + 1))
            || (second_half && starts_dotdot(&chars, i + 1))
    })
}

/// Every string of the document, keys included. A loop, not recursion, so the depth cannot overflow the stack.
fn strings(value: &Value) -> Vec<&str> {
    let mut found = Vec::new();
    let mut stack = vec![value];
    while let Some(item) = stack.pop() {
        match item {
            Value::String(text) => found.push(text.as_str()),
            Value::Object(map) => {
                for (key, inner) in map {
                    found.push(key.as_str());
                    stack.push(inner);
                }
            }
            Value::Array(list) => stack.extend(list),
            _ => {}
        }
    }
    found
}

/// A string of 1..=`max` characters.
fn is_plain(value: Option<&Value>, max: usize) -> bool {
    value
        .and_then(Value::as_str)
        .is_some_and(|s| !s.is_empty() && s.chars().count() <= max)
}

/// A JSON null counts as absent, as `None` does in the reference.
fn present<'a>(doc: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    doc.get(key).filter(|v| !v.is_null())
}

fn integer(value: &Value) -> Option<i128> {
    let n = value.as_number()?;
    n.as_i64()
        .map(i128::from)
        .or_else(|| n.as_u64().map(i128::from))
}

fn check_file(file: Option<&Value>, out: &mut Vec<String>) {
    let Some(file) = file.and_then(Value::as_str).filter(|s| !s.is_empty()) else {
        out.push("file is missing".into());
        return;
    };
    if file.chars().count() > MAX_PATH {
        out.push(format!("file is longer than {MAX_PATH}"));
    } else if leaves_repo(file) {
        out.push("file leaves the repository".into());
    } else if file.contains(['\\', ':', '\0']) {
        out.push("file is not a plain forward-slash path".into());
    } else if file.split('/').any(|part| part.is_empty() || part == ".") {
        out.push("file is not canonical (empty or . segment)".into());
    }
}

/// `docmap` and `telemetry`: an object whose fields are plain non-empty strings; the first `required` are required.
fn check_pair(
    doc: &Map<String, Value>,
    key: &str,
    fields: [&str; 2],
    required: usize,
    out: &mut Vec<String>,
) {
    let Some(value) = present(doc, key) else {
        return;
    };
    let Some(map) = value.as_object() else {
        out.push(format!("{key} is not an object"));
        return;
    };
    for (index, field) in fields.iter().enumerate() {
        let text = present(map, field);
        if text.is_none() && index >= required {
            continue;
        }
        let name = format!("{key}.{field}");
        if !is_plain(text, limit(&name)) {
            out.push(format!("{name} is missing or longer than {}", limit(&name)));
        }
    }
    if key == "docmap"
        && map
            .get("node")
            .and_then(Value::as_str)
            .is_some_and(leaves_repo)
    {
        out.push("docmap.node leaves the repository".into());
    }
}

/// Every reason to refuse `raw`, empty when the reader may use the reference.
#[must_use]
pub fn problems(raw: &[u8]) -> Vec<String> {
    if raw.len() > MAX_BYTES {
        return vec![format!("larger than {MAX_BYTES} bytes")];
    }
    let Some(parsed) = std::str::from_utf8(raw)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
    else {
        return vec!["not JSON in UTF-8".into()];
    };
    let Value::Object(doc) = &parsed else {
        return vec!["not a JSON object".into()];
    };
    if doc.get("schema").and_then(Value::as_str) != Some(SCHEMA) {
        let shown = doc
            .get("schema")
            .map_or_else(|| "null".to_owned(), Value::to_string);
        return vec![format!("schema {shown} is not {SCHEMA}")];
    }
    let mut out = Vec::new();
    if !doc.get("repo").and_then(Value::as_str).is_some_and(is_repo) {
        out.push("repo is missing or not a plain name".into());
    }
    check_file(doc.get("file"), &mut out);
    match doc
        .get("lines")
        .and_then(Value::as_array)
        .filter(|l| l.len() == 2)
    {
        Some(lines) if lines.iter().all(|n| integer(n).is_some()) => {
            let (first, last) = (
                integer(&lines[0]).unwrap_or(0),
                integer(&lines[1]).unwrap_or(0),
            );
            if !(1 <= first && first <= last && last <= MAX_LINE) {
                out.push(format!("lines is not 1 <= first <= last <= {MAX_LINE}"));
            }
        }
        _ => out.push("lines is not two integers".into()),
    }
    if present(doc, "sha").is_some_and(|v| !v.as_str().is_some_and(is_sha)) {
        out.push("sha is not 40 or 64 lowercase hex characters".into());
    }
    for key in ["dirty", "truncated"] {
        if doc.get(key).is_some_and(|v| !v.is_boolean()) {
            out.push(format!("{key} is not a boolean"));
        }
    }
    if present(doc, "lang").is_some_and(|v| !v.as_str().is_some_and(is_lang)) {
        out.push("lang is not a plain name".into());
    }
    if let Some(symbol) = present(doc, "symbol")
        && !is_plain(Some(symbol), limit("symbol"))
    {
        out.push(format!(
            "symbol is empty or longer than {}",
            limit("symbol")
        ));
    }
    if let Some(excerpt) = present(doc, "excerpt") {
        match excerpt.as_str() {
            None => out.push("excerpt is not a string".into()),
            Some(text) if text.len() > MAX_EXCERPT_BYTES => {
                out.push(format!("excerpt is longer than {MAX_EXCERPT_BYTES} bytes"));
            }
            Some(_) => {}
        }
    }
    if doc
        .get("class")
        .is_some_and(|v| !v.as_str().is_some_and(|c| CLASSES.contains(&c)))
    {
        out.push("class is not public or internal".into());
    }
    check_pair(doc, "docmap", ["node", "fn"], 1, &mut out);
    check_pair(doc, "telemetry", ["namespace", "key"], 2, &mut out);
    if strings(&parsed)
        .iter()
        .any(|text| text.chars().any(is_control))
    {
        out.push("a field holds a control character".into());
    }
    out
}

/// The opening of a fence: up to three spaces, three or more backticks, then an info string without a backtick.
/// Returns the number of backticks and the info string.
fn fence_open(line: &str) -> Option<(usize, &str)> {
    let rest = line.trim_start_matches(' ');
    if line.len() - rest.len() > 3 {
        return None;
    }
    let ticks = rest.chars().take_while(|&c| c == '`').count();
    let info = &rest[ticks..];
    (ticks >= 3 && !info.contains('`')).then_some((ticks, info))
}

/// The closing of a fence of `fence` backticks: up to three spaces, at least as many backticks, then white space only.
fn fence_close(line: &str, fence: usize) -> bool {
    let rest = line.trim_start_matches(' ');
    if line.len() - rest.len() > 3 {
        return false;
    }
    let ticks = rest.chars().take_while(|&c| c == '`').count();
    ticks >= fence && rest[ticks..].chars().all(is_space)
}

/// The text of every closed fenced block of language `code-ref` in the body of a task.
///
/// A fence opens on a line of three or more backticks and closes on a line of at least as many; a block of any other
/// language, and anything inside it, is skipped, so a `code-ref` shown inside a longer fence is not a reference. The
/// frontmatter is not body. An unclosed block is dropped.
#[must_use]
pub fn blocks(markdown: &str) -> Vec<String> {
    let text = markdown.replace("\r\n", "\n");
    let lines: Vec<&str> = text.split('\n').collect();
    let mut start = 0;
    if lines.first().is_some_and(|l| l.trim_end() == "---")
        && let Some(end) = lines.iter().skip(1).position(|l| l.trim_end() == "---")
    {
        start = end + 2;
    }
    let mut found = Vec::new();
    let (mut fence, mut mine, mut body): (Option<usize>, bool, Vec<&str>) =
        (None, false, Vec::new());
    for line in lines.iter().skip(start) {
        match fence {
            None => {
                if let Some((ticks, info)) = fence_open(line) {
                    (fence, mine, body) = (Some(ticks), info.trim() == FENCE_LANG, Vec::new());
                }
            }
            Some(ticks) if fence_close(line, ticks) => {
                if mine {
                    found.push(body.join("\n"));
                }
                fence = None;
            }
            Some(_) if mine => body.push(line),
            Some(_) => {}
        }
    }
    found
}

/// `(index, problems)` for every `code-ref` block of a task; empty problems means usable.
#[must_use]
pub fn task_problems(markdown: &str) -> Vec<(usize, Vec<String>)> {
    blocks(markdown)
        .iter()
        .enumerate()
        .map(|(i, text)| (i, problems(text.as_bytes())))
        .collect()
}

#[cfg(test)]
mod tests;
