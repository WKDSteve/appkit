#!/usr/bin/env python3
"""The target's check of a wkd-intent/1 (schemas/wkd-intent/1/README.md): the reference apps port and test against.

    python tools/check-wkd-intent.py <file> [--actions=tasks.new,tasks.show]

An intent is untrusted input: `problems` never raises on one, it names every problem. Exit 0 when there are none, 1 when
there are, 2 for a wrong call.
"""

import json
import re
import sys

MAX_BYTES = 64 * 1024
SCHEMA = "wkd-intent/1"
ACTIONS = ("tasks.new", "tasks.show", "rules.show", "rules.new", "ai.handoff")
MODES = ("form", "ai-draft", "agent")
SHOW = ("tasks.show", "rules.show")
CLASSES = ("public", "internal")
LIMITS = {"title": 120, "excerpt": 4096, "repro": 1000}
MAX_PATH = 300
RUN_URL = "https://github.com/"

# Control characters other than tab, line break and carriage return, and the C1 block.
CONTROL = re.compile("[\x00-\x08\x0b\x0c\x0e-\x1f\x7f-\x9f]")
# One word that leaves the repository: an absolute path, a home, a `..` segment. Quotes and `=` start a word too, so
# `--out="/etc/x"` and `--out=/etc/x` are seen.
ESCAPE = re.compile(r"""(?:^|[\s"'=])(?:[/\\]|[A-Za-z]:[/\\]|~[/\\]|\$HOME\b|%[A-Za-z]+%)|(?:^|[\s"'=/\\])\.\.(?:[/\\]|\s|$)""")


def leaves_repo(text):
    return ESCAPE.search(text) is not None


def _strings(value):
    if isinstance(value, str):
        yield value
    elif isinstance(value, dict):
        for key, item in value.items():
            yield key
            yield from _strings(item)
    elif isinstance(value, list):
        for item in value:
            yield from _strings(item)


def _too_long(text, limit):
    return len(text) > limit


def _check_finding(finding, out):
    if finding.get("schema") != "testing-finding/1":
        out.append("finding.schema is not testing-finding/1")
    fp = finding.get("fp")
    if not (isinstance(fp, str) and re.fullmatch("[0-9a-f]{16}", fp)):
        out.append("finding.fp is not 16 hex characters")
    if finding.get("class") not in CLASSES:
        out.append("finding.class is not public or internal")
    title = finding.get("title")
    if not (isinstance(title, str) and title):
        out.append("finding.title is missing")
    if not isinstance(finding.get("excerpt"), str):
        out.append("finding.excerpt is missing")
    for key, limit in LIMITS.items():
        text = finding.get(key)
        if isinstance(text, str) and _too_long(text, limit):
            out.append(f"finding.{key} is longer than {limit}")
    repro = finding.get("repro")
    if repro is not None and not isinstance(repro, str):
        out.append("finding.repro is not a string")
    elif isinstance(repro, str) and leaves_repo(repro):
        out.append("finding.repro leaves the repository")
    where = finding.get("where")
    if where is not None:
        file = where.get("file") if isinstance(where, dict) else None
        if not isinstance(file, str):
            out.append("finding.where.file is missing")
        elif _too_long(file, MAX_PATH):
            out.append(f"finding.where.file is longer than {MAX_PATH}")
        elif leaves_repo(file):
            out.append("finding.where.file leaves the repository")
    run = finding.get("run")
    url = run.get("url") if isinstance(run, dict) else None
    if url is not None and not (isinstance(url, str) and url.startswith(RUN_URL)):
        out.append(f"finding.run.url does not start with {RUN_URL}")


def problems(raw, accepted=ACTIONS):
    """Every reason to refuse `raw` (bytes), empty when the target may show its form."""
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
    action, mode = doc.get("action"), doc.get("mode")
    # An unhashable value is no action either: `in` on a tuple compares, it does not hash.
    if action not in ACTIONS:
        out.append("unknown action")
    elif action not in accepted:
        out.append(f"this target does not take {action}")
    if mode not in MODES:
        out.append("unknown mode")
    elif action in SHOW and mode != "form":
        out.append(f"{action} only knows mode form")
    source = doc.get("source")
    app = source.get("app") if isinstance(source, dict) else None
    if not (isinstance(app, str) and re.fullmatch("[a-z][a-z0-9-]{0,63}", app)):
        out.append("source.app is missing or not a plain name")
    for key in ("finding", "target"):
        if key not in doc:
            out.append(f"{key} is missing")
    finding, target = doc.get("finding"), doc.get("target")
    if action in SHOW:
        if finding is not None:
            out.append(f"{action} carries no finding")
        tid = target.get("id") if isinstance(target, dict) else None
        if not (isinstance(tid, str) and tid):
            out.append("target.id is missing")
        elif _too_long(tid, MAX_PATH):
            out.append(f"target.id is longer than {MAX_PATH}")
        elif leaves_repo(tid):
            out.append("target.id leaves the repository")
    elif action in ACTIONS:
        if target is not None:
            out.append(f"{action} has no target")
        if isinstance(finding, dict):
            _check_finding(finding, out)
        else:
            out.append(f"{action} needs a finding")
    if any(CONTROL.search(text) for text in _strings(doc)):
        out.append("a field holds a control character")
    return out


def main(argv):
    accepted = ACTIONS
    files = []
    for arg in argv:
        if arg.startswith("--actions="):
            accepted = tuple(arg.split("=", 1)[1].split(","))
        elif arg.startswith("-"):
            print(f"unknown option {arg}", file=sys.stderr)
            return 2
        else:
            files.append(arg)
    if len(files) != 1:
        print("usage: check-wkd-intent.py <file> [--actions=a,b]", file=sys.stderr)
        return 2
    with open(files[0], "rb") as handle:
        found = problems(handle.read(MAX_BYTES + 1), accepted)
    for line in found:
        print(line)
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
