#!/usr/bin/env python3
"""Decide which commit the rolling build `latest` is made from (rule GIT-08).

    pick_target.py --repo OWNER/NAME --branch main --ci-workflow ci.yml [--requested SHA] [--built SHA]

`latest` is the newest commit of the branch whose CI run is green, not the tip: a tip that is red, or whose
CI is still running, must not keep the newest green commit from being built. A channel only moves forward:
a target that is not a descendant of the commit `version.json` of the built `latest` names builds nothing.

The decision is `pick()`, a pure function (tested in test_pick_target.py); `main()` only gathers its
input through `gh`. Output, one `key=value` per line (for $GITHUB_OUTPUT): `target=<40 hex>` (empty when
nothing is to be built) and `reason=<words>`.
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from typing import Callable, Iterable

FULL = re.compile(r"^[0-9a-f]{40}$")
# How far down the branch the newest green commit is looked for: a branch whose last 30 commits are all
# red or still running builds nothing, and says so.
WINDOW = 30


def pick(
    commits: Iterable[str],
    green: Iterable[str],
    built: str | None,
    is_descendant: Callable[[str, str], bool],
    requested: str | None = None,
) -> tuple[str | None, str]:
    """The commit to build, or None, and why.

    commits: the branch, newest first. green: commits whose CI run succeeded. built: the commit of the
    `latest` that is up now (None: there is none). is_descendant(a, b): b is a or has a as an ancestor.
    requested: a commit named by hand; it must be green and on the branch, and it may rebuild `built`.
    """
    commits = list(commits)
    green = set(green)
    if requested:
        if requested not in commits:
            return None, f"{requested[:7]} is not among the newest {len(commits)} commits of the branch"
        if requested not in green:
            return None, f"{requested[:7]} has no green CI run"
        target = requested
    else:
        target = next((c for c in commits if c in green), None)
        if target is None:
            return None, f"none of the newest {len(commits)} commits has a green CI run"
    if built is None:
        return target, f"no build yet: {target[:7]}"
    if target == built:
        if requested:
            return target, f"{target[:7]} is built again by hand"
        return None, f"{target[:7]} is the newest green commit and is built already"
    if not is_descendant(built, target):
        return None, f"{target[:7]} is not a descendant of the built {built[:7]}: latest only moves forward"
    return target, f"newest green commit {target[:7]} (built: {built[:7]})"


def gh(*args: str) -> str:
    return subprocess.run(["gh", *args], check=True, capture_output=True, text=True).stdout


def main(argv: list[str]) -> int:
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("--repo", required=True)
    p.add_argument("--branch", default="main")
    p.add_argument("--ci-workflow", default="ci.yml")
    p.add_argument("--requested", default="")
    args = p.parse_args(argv)
    if args.requested and not FULL.match(args.requested):
        print(f"::error::sha must be a full commit id, got '{args.requested}'", file=sys.stderr)
        return 1
    repo = args.repo
    commits = gh("api", f"repos/{repo}/commits?sha={args.branch}&per_page={WINDOW}", "--jq", ".[].sha").split()
    runs = gh(
        "api",
        f"repos/{repo}/actions/workflows/{args.ci_workflow}/runs?branch={args.branch}&event=push"
        "&status=completed&per_page=100",
        "--jq", '.workflow_runs[] | select(.conclusion == "success") | .head_sha',
    ).split()
    built = None
    try:
        doc = json.loads(gh("release", "download", "latest", "--repo", repo, "--pattern", "version.json", "--output", "-"))
        if isinstance(doc.get("commit"), str) and FULL.match(doc["commit"]):
            built = doc["commit"]
    except (subprocess.CalledProcessError, ValueError):
        built = None  # no release yet, or one that predates version.json: nothing to compare with

    def is_descendant(old: str, new: str) -> bool:
        try:
            status = gh("api", f"repos/{repo}/compare/{old}...{new}", "--jq", ".status").strip()
        except subprocess.CalledProcessError:
            return False  # the old commit is unknown here (a rewritten history): do not move onto a guess
        return status in ("ahead", "identical")

    target, reason = pick(commits, runs, built, is_descendant, args.requested or None)
    print(f"target={target or ''}")
    print(f"reason={reason}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
