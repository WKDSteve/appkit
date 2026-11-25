#!/usr/bin/env python3
"""Fail when a tracked file names a rule of a private ruleset by its id.

appkit is public. The rules behind its code live in a ruleset of the maintainer that
the reader does not have; an id such as `PREFIX-12` in a comment, a test name, a doc or
a message tells the reader nothing, so the text says what the rule says. This reads
every tracked text file and reports each place where one of the ruleset prefixes is
followed by a dash and two digits:

    python tools/check-rule-ids.py [--root DIR]

Not looked at: lock files, and files that are not UTF-8 text. An id-like word that is
no rule (a version such as `UTF-16`, the id of a rule in a test fixture of an app's data)
has a prefix that is not on the list below, so it is not found.

Exit 0: clean. Exit 1: the places are listed. Exit 2: git could not be asked.
"""

import argparse
import re
import subprocess
import sys

# The prefixes of the rules of the maintainer's rulesets (arbeit, web, native, mobile, rust, release).
PREFIXES = (
    "ACC", "ALLG", "AND", "APG", "APP", "BASE", "CI", "CSS", "DOKU", "FALCON", "FORM", "GIT", "HTML",
    "JS", "MASCHINE", "MOB", "MREL", "MSEC", "MUI", "NREL", "ORCH", "PWA", "REVIEW", "RS", "SRV",
    "TASKS", "WEB", "WPERF",
)
RULE_ID = re.compile(r"\b(?:" + "|".join(PREFIXES) + r")-\d\d\b")
SKIPPED_SUFFIXES = (".lock", "pnpm-lock.yaml", "package-lock.json")


def tracked_files(root):
    try:
        out = subprocess.run(
            ["git", "ls-files", "-z"], check=True, capture_output=True, cwd=root
        ).stdout
    except (OSError, subprocess.CalledProcessError) as error:
        print(f"cannot list the tracked files: {error}", file=sys.stderr)
        sys.exit(2)
    return [p.decode("utf-8", "surrogateescape") for p in out.split(b"\0") if p]


def findings(text):
    """The (line number, rule id) pairs of one file's text."""
    found = []
    for number, line in enumerate(text.split("\n"), start=1):
        for match in RULE_ID.finditer(line):
            found.append((number, match.group(0)))
    return found


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--root", default=".")
    args = parser.parse_args(argv)
    problems = []
    for path in tracked_files(args.root):
        if path.endswith(SKIPPED_SUFFIXES):
            continue
        try:
            with open(f"{args.root}/{path}", encoding="utf-8") as handle:
                text = handle.read()
        except (OSError, UnicodeDecodeError):
            continue
        for number, rule in findings(text):
            problems.append(f"{path}:{number}: names the rule {rule} by its id: say what the rule says")
    for problem in problems:
        print(problem)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
