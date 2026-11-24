#!/usr/bin/env python3
"""Decide which commit the rolling build `latest` is made from (rule GIT-08).

    pick_target.py --repo OWNER/NAME --branch main --ci-workflow ci.yml [--channel latest|nightly|stable]
                   [--channels "latest nightly stable"] [--requested SHA]

Three rolling channels (rule GIT-08), each only moving forward: a target that is not a descendant of the commit
`version.json` of the channel's own release names builds nothing.

  latest   the newest commit of the branch whose CI run is green, not the tip: a tip that is red, or whose CI is
           still running, must not keep the newest green commit from being built.
  nightly  the tip of the branch, built at once, green or not (for those who want the newest state before the CI
           is through).
  stable   the commit `latest` has had for at least STABLE_DAYS days, if the CI did not turn red and no bug was
           filed since. `--requested` promotes a green commit by hand, whatever its age.

`--channels` is what the app has switched on (default: latest); a channel that is not on builds nothing.

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
from datetime import datetime, timedelta, timezone
from typing import Callable, Iterable

FULL = re.compile(r"^[0-9a-f]{40}$")
# How far down the branch the newest green commit is looked for: a branch whose last 30 commits are all
# red or still running builds nothing, and says so.
WINDOW = 30
CHANNELS = ("latest", "nightly", "stable")
# How long `latest` has to stay at the top, unchanged, before `stable` takes it.
STABLE_DAYS = 14


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


def pick_nightly(
    commits: Iterable[str],
    built: str | None,
    is_descendant: Callable[[str, str], bool],
    requested: str | None = None,
) -> tuple[str | None, str]:
    """`nightly`: the tip of the branch (or the commit named by hand), green or not, only forward."""
    commits = list(commits)
    if not commits:
        return None, "the branch has no commits"
    target = requested or commits[0]
    if target not in commits:
        return None, f"{target[:7]} is not among the newest {len(commits)} commits of the branch"
    return _forward(target, built, is_descendant, bool(requested))


def pick_stable(
    commits: Iterable[str],
    green: Iterable[str],
    latest: tuple[str, datetime] | None,
    built: str | None,
    is_descendant: Callable[[str, str], bool],
    now: datetime,
    red_since: bool = False,
    bug_since: bool = False,
    requested: str | None = None,
) -> tuple[str | None, str]:
    """`stable`: the commit `latest` has had for STABLE_DAYS days, if nothing went wrong since.

    latest: (commit, time it was published) read from the `version.json` of the release `latest`; its date is
    the moment the files of that commit went up, so it is also how long that commit has been at the top.
    red_since: a CI run of the branch went red after that moment. bug_since: a bug was filed after it.
    requested: a green commit of the branch promoted by hand; it skips the waiting and the two checks.
    """
    commits = list(commits)
    green = set(green)
    if requested:
        if requested not in commits:
            return None, f"{requested[:7]} is not among the newest {len(commits)} commits of the branch"
        if requested not in green:
            return None, f"{requested[:7]} has no green CI run"
        return _forward(requested, built, is_descendant, True)
    if latest is None:
        return None, "there is no latest build to take"
    commit, since = latest
    if now - since < timedelta(days=STABLE_DAYS):
        return None, f"latest ({commit[:7]}) has been up since {since:%Y-%m-%d}: less than {STABLE_DAYS} days"
    if red_since:
        return None, f"the CI turned red after latest ({commit[:7]}) went up"
    if bug_since:
        return None, f"a bug was filed after latest ({commit[:7]}) went up"
    return _forward(commit, built, is_descendant, False)


def _forward(target: str, built: str | None, is_descendant: Callable[[str, str], bool], by_hand: bool):
    if built is None:
        return target, f"no build yet: {target[:7]}"
    if target == built:
        if by_hand:
            return target, f"{target[:7]} is built again by hand"
        return None, f"{target[:7]} is built already"
    if not is_descendant(built, target):
        return None, f"{target[:7]} is not a descendant of the built {built[:7]}: the channel only moves forward"
    return target, f"{target[:7]} (built: {built[:7]})"


def ci_runs_url(repo: str, workflow: str, branch: str) -> str:
    """The API path of the push runs of the CI workflow on `branch`.

    No `status=completed` filter: GitHub leaves out runs that are completed and green (tasks-desktop 6ef892c on
    2026-11-24), which made `latest` build an older commit. A run with a conclusion is completed; the caller
    selects on the conclusion."""
    return f"repos/{repo}/actions/workflows/{workflow}/runs?branch={branch}&event=push&per_page=100"


def gh(*args: str) -> str:
    return subprocess.run(["gh", *args], check=True, capture_output=True, text=True).stdout


def release_version(repo: str, tag: str) -> dict | None:
    """The `version.json` of the release `tag`, or None (no release yet, or one that predates version.json)."""
    try:
        doc = json.loads(gh("release", "download", tag, "--repo", repo, "--pattern", "version.json", "--output", "-"))
    except (subprocess.CalledProcessError, ValueError):
        return None
    return doc if isinstance(doc, dict) else None


def built_commit(doc: dict | None) -> str | None:
    if doc and isinstance(doc.get("commit"), str) and FULL.match(doc["commit"]):
        return doc["commit"]
    return None


def parse_time(text: str) -> datetime | None:
    try:
        return datetime.strptime(text, "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=timezone.utc)
    except (TypeError, ValueError):
        return None


def main(argv: list[str]) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--repo", required=True)
    p.add_argument("--branch", default="main")
    p.add_argument("--ci-workflow", default="ci.yml")
    p.add_argument("--channel", default="latest", choices=CHANNELS)
    p.add_argument("--channels", default="latest", help="the channels the app has switched on, separated by spaces")
    p.add_argument("--requested", default="")
    args = p.parse_args(argv)
    if args.requested and not FULL.match(args.requested):
        print(f"::error::sha must be a full commit id, got '{args.requested}'", file=sys.stderr)
        return 1
    on = args.channels.replace(",", " ").split()
    unknown = [c for c in on if c not in CHANNELS]
    if unknown:
        print(f"::error::channels names {' '.join(unknown)}; the channels are {' '.join(CHANNELS)}", file=sys.stderr)
        return 1
    if args.channel not in on:
        print("target=")
        print(f"reason=the channel {args.channel} is not switched on for this app (channels: {' '.join(on) or 'none'})")
        return 0
    repo = args.repo
    commits = gh("api", f"repos/{repo}/commits?sha={args.branch}&per_page={WINDOW}", "--jq", ".[].sha").split()

    def ci_runs(conclusion: str, field: str) -> list[str]:
        return gh(
            "api",
            ci_runs_url(repo, args.ci_workflow, args.branch),
            "--jq", f'.workflow_runs[] | select(.conclusion == "{conclusion}") | .{field}',
        ).split()

    def is_descendant(old: str, new: str) -> bool:
        try:
            status = gh("api", f"repos/{repo}/compare/{old}...{new}", "--jq", ".status").strip()
        except subprocess.CalledProcessError:
            return False  # the old commit is unknown here (a rewritten history): do not move onto a guess
        return status in ("ahead", "identical")

    built = built_commit(release_version(repo, args.channel))  # None: nothing to compare with
    requested = args.requested or None
    if args.channel == "latest":
        target, reason = pick(commits, ci_runs("success", "head_sha"), built, is_descendant, requested)
    elif args.channel == "nightly":
        target, reason = pick_nightly(commits, built, is_descendant, requested)
    else:
        doc = release_version(repo, "latest")
        latest = None
        if built_commit(doc) and parse_time(doc.get("date", "")):
            latest = (built_commit(doc), parse_time(doc["date"]))
        red = bug = False
        if latest:
            since = latest[1]
            red = any(parse_time(c) and parse_time(c) > since for c in ci_runs("failure", "created_at"))
            try:
                # Issues only: a pull request is an issue too, and carries the key `pull_request`.
                made = gh("api", f"repos/{repo}/issues?labels=bug&state=all&per_page=100",
                          "--jq", ".[] | select(.pull_request | not) | .created_at").split()
                bug = any(parse_time(c) and parse_time(c) > since for c in made)
            except subprocess.CalledProcessError:
                bug = False  # issues are switched off here: the bug tasks of the maintainer are not visible to CI
        target, reason = pick_stable(commits, ci_runs("success", "head_sha"), latest, built, is_descendant,
                                     datetime.now(timezone.utc), red, bug, requested)
    print(f"target={target or ''}")
    print(f"reason={reason}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
