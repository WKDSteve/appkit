#!/usr/bin/env python3
"""The reader's check of a code-ref/1 (schemas/code-ref/1/README.md): the reference apps port and test against.

    python tools/check-code-ref.py <file.json>
    python tools/check-code-ref.py --task <task.md>

The second form finds every fenced block of language `code-ref` in the body of a task and checks each one. A code-ref
is untrusted input: `problems` never raises on one, it names every problem. Exit 0 when there are none, 1 when there
are, 2 for a wrong call.
"""

import json
import re
import sys

MAX_BYTES = 64 * 1024
MAX_EXCERPT_BYTES = 4096
SCHEMA = "code-ref/1"
FENCE_LANG = "code-ref"
CLASSES = ("public", "internal")
MAX_PATH = 300
MAX_LINE = 10_000_000
LIMITS = {"symbol": 200, "docmap.node": 300, "docmap.fn": 200, "telemetry.namespace": 100, "telemetry.key": 300}

REPO = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{0,99}")
SHA = re.compile(r"(?:[0-9a-f]{40}|[0-9a-f]{64})")
LANG = re.compile(r"[a-z0-9][a-z0-9+#._-]{0,31}")

# Control characters other than tab, line break and carriage return, and the C1 block.
CONTROL = re.compile("[\x00-\x08\x0b\x0c\x0e-\x1f\x7f-\x9f]")
# One word that leaves the repository: an absolute path, a home, a `..` segment (as in check-wkd-intent.py).
ESCAPE = re.compile(r"""(?:^|[\s"'=])(?:[/\\]|[A-Za-z]:[/\\]|~[/\\]|\$\{?HOME\b|%[A-Za-z]+%)|(?:^|[\s"'=/\\])\.\.(?:[/\\]|\s|$)""")


def leaves_repo(text):
    return ESCAPE.search(text) is not None


def _strings(value):
    # A loop, not recursion: what the JSON parser just managed to nest must not overflow here.
    stack = [value]
    while stack:
        item = stack.pop()
        if isinstance(item, str):
            yield item
        elif isinstance(item, dict):
            stack.extend(item.keys())
            stack.extend(item.values())
        elif isinstance(item, list):
            stack.extend(item)


def _is_int(value):
    # `True` is an int in Python; a flag is no line number.
    return isinstance(value, int) and not isinstance(value, bool)


def _plain(value, limit):
    return isinstance(value, str) and 0 < len(value) <= limit


def check_file(file, out):
    """`file` is one canonical repository-relative path with forward slashes and no colon."""
    if not isinstance(file, str) or not file:
        out.append("file is missing")
    elif len(file) > MAX_PATH:
        out.append(f"file is longer than {MAX_PATH}")
    elif leaves_repo(file):
        out.append("file leaves the repository")
    elif "\\" in file or ":" in file or "\0" in file:
        out.append("file is not a plain forward-slash path")
    elif any(part in ("", ".") for part in file.split("/")):
        out.append("file is not canonical (empty or . segment)")


def _check_pair(doc, key, fields, required, out):
    """`docmap` and `telemetry`: an object whose fields are plain non-empty strings of a limit; `required` of them are required."""
    value = doc.get(key)
    if value is None:
        return
    if not isinstance(value, dict):
        out.append(f"{key} is not an object")
        return
    for index, field in enumerate(fields):
        text = value.get(field)
        name = f"{key}.{field}"
        if text is None and index >= required:
            continue
        if not _plain(text, LIMITS[name]):
            out.append(f"{name} is missing or longer than {LIMITS[name]}")
    node = value.get("node")
    if key == "docmap" and isinstance(node, str) and leaves_repo(node):
        out.append("docmap.node leaves the repository")


def problems(raw):
    """Every reason to refuse `raw` (bytes), empty when the reader may use the reference."""
    if len(raw) > MAX_BYTES:
        return [f"larger than {MAX_BYTES} bytes"]
    try:
        doc = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, ValueError, RecursionError):
        return ["not JSON in UTF-8"]
    if not isinstance(doc, dict):
        return ["not a JSON object"]
    schema = doc.get("schema")
    if schema != SCHEMA:
        return [f"schema {schema!r} is not {SCHEMA}"]
    out = []
    repo = doc.get("repo")
    if not (isinstance(repo, str) and REPO.fullmatch(repo)):
        out.append("repo is missing or not a plain name")
    check_file(doc.get("file"), out)
    lines = doc.get("lines")
    if not (isinstance(lines, list) and len(lines) == 2 and all(_is_int(n) for n in lines)):
        out.append("lines is not two integers")
    elif not (1 <= lines[0] <= lines[1] <= MAX_LINE):
        out.append(f"lines is not 1 <= first <= last <= {MAX_LINE}")
    sha = doc.get("sha")
    if sha is not None and not (isinstance(sha, str) and SHA.fullmatch(sha)):
        out.append("sha is not 40 or 64 lowercase hex characters")
    if "dirty" in doc and not isinstance(doc["dirty"], bool):
        out.append("dirty is not a boolean")
    if "truncated" in doc and not isinstance(doc["truncated"], bool):
        out.append("truncated is not a boolean")
    lang = doc.get("lang")
    if lang is not None and not (isinstance(lang, str) and LANG.fullmatch(lang)):
        out.append("lang is not a plain name")
    symbol = doc.get("symbol")
    if symbol is not None and not _plain(symbol, LIMITS["symbol"]):
        out.append(f"symbol is empty or longer than {LIMITS['symbol']}")
    excerpt = doc.get("excerpt")
    if excerpt is not None:
        if not isinstance(excerpt, str):
            out.append("excerpt is not a string")
        elif len(excerpt.encode("utf-8", "surrogatepass")) > MAX_EXCERPT_BYTES:
            out.append(f"excerpt is longer than {MAX_EXCERPT_BYTES} bytes")
    if "class" in doc and doc["class"] not in CLASSES:
        out.append("class is not public or internal")
    _check_pair(doc, "docmap", ("node", "fn"), 1, out)
    _check_pair(doc, "telemetry", ("namespace", "key"), 2, out)
    if any(CONTROL.search(text) for text in _strings(doc)):
        out.append("a field holds a control character")
    return out


_OPEN = re.compile(r"^ {0,3}(`{3,})([^`]*)$")


def blocks(markdown):
    """The text of every closed fenced block of language `code-ref` in the body of a task (a str).

    A fence opens on a line of three or more backticks and closes on a line of at least as many; a block of any other
    language, and anything inside it, is skipped, so a `code-ref` shown inside a longer fence is not a reference. The
    frontmatter is not body. An unclosed block is dropped.
    """
    lines = markdown.replace("\r\n", "\n").split("\n")
    start = 0
    if lines and lines[0].rstrip() == "---":
        for index in range(1, len(lines)):
            if lines[index].rstrip() == "---":
                start = index + 1
                break
    found, fence, mine, body = [], None, False, []
    for line in lines[start:]:
        if fence is None:
            match = _OPEN.match(line)
            if match:
                fence, mine, body = len(match.group(1)), match.group(2).strip() == FENCE_LANG, []
        elif re.match(r"^ {0,3}`{%d,}\s*$" % fence, line):
            if mine:
                found.append("\n".join(body))
            fence = None
        elif mine:
            body.append(line)
    return found


def task_problems(markdown):
    """`[(index, [problems])]` for every `code-ref` block of a task; empty problems means usable."""
    return [(i, problems(text.encode("utf-8"))) for i, text in enumerate(blocks(markdown))]


def main(argv):
    task = "--task" in argv
    files = [a for a in argv if not a.startswith("-")]
    if len(files) != 1 or any(a.startswith("-") and a != "--task" for a in argv):
        print("usage: check-code-ref.py [--task] <file>", file=sys.stderr)
        return 2
    with open(files[0], "rb") as handle:
        data = handle.read(None if task else MAX_BYTES + 1)
    if not task:
        found = problems(data)
        for line in found:
            print(line)
        return 1 if found else 0
    bad = 0
    for index, found in task_problems(data.decode("utf-8", "replace")):
        for line in found:
            print(f"block {index + 1}: {line}")
        bad += bool(found)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
