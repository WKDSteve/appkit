#!/usr/bin/env python3
"""Move the apps that use appkit to one tag of appkit, in one command.

    python tools/bump-apps.py v0.23.2                       all five apps, files edited, lock files refreshed, pins checked
    python tools/bump-apps.py v0.23.2 --apps ai-desktop     only these (comma separated)
    python tools/bump-apps.py v0.23.2 --dry-run             say what would change, change nothing
    python tools/bump-apps.py v0.23.2 --commit              also commit each app (never pushes)

Why a script: an app pins appkit in several places that must agree (and its own check fails when they do
not), and doing it by hand is what makes a small appkit fix cost an afternoon.

  kit only (ai-desktop, docmap-desktop)   the `uses: WKDSteve/appkit/release-kit/...@<sha> # vX.Y.Z` lines of the
                                          workflows and tools/release-kit.pin
  full (tasks-desktop, rules-desktop, hub-desktop)
                                          the same, and the tag in Cargo.toml and ui/package.json (appkit-ui and appkit-web); then
                                          `cargo update -p appkit-hub` and `pnpm install` rewrite the lock files

The tag must exist in this clone AND on origin, and name the same commit in both: a pin to a commit that GitHub
does not know turns every build red. An app whose files to edit have uncommitted changes of their own is
skipped (another session's work is never swept into a bump commit). When done, each app's own pin check runs
(tools/check-appkit-pin.py or tools/check-release-kit-pin.py); the exit code is 1 when one of them fails.
For work on appkit and an app at the same time, do not bump: use a `[patch]` in an uncommitted
.cargo/config.toml (see the CLAUDE.md of tasks-desktop).

Not touched on purpose: the separate pin of the design tokens of docmap-desktop (tools/appkit-ui.pin, a checked-in
copy of a file of appkit-ui at its own tag, moved by hand with tools/check-appkit-tokens.py).
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

APPS = {
    "ai-desktop": "kit",
    "docmap-desktop": "kit",
    "tasks-desktop": "full",
    "rules-desktop": "full",
    "hub-desktop": "full",
}

TAG = re.compile(r"^v\d+\.\d+\.\d+$")
USES = re.compile(r"(uses:\s*WKDSteve/appkit/release-kit/[A-Za-z0-9_./-]+@)[0-9a-f]{40}([ \t]*#[ \t]*)v\d+\.\d+\.\d+")
# (?=[ \t]*\r?$): the line may end in CRLF, and the end of the line is not part of what is replaced
PIN_TAG = re.compile(r"^(tag\s*=\s*)v\d+\.\d+\.\d+(?=[ \t]*\r?$)", re.M)
PIN_COMMIT = re.compile(r"^(commit\s*=\s*)[0-9a-f]{40}(?=[ \t]*\r?$)", re.M)
CARGO = re.compile(r'(git\s*=\s*"https://github\.com/WKDSteve/appkit",\s*tag\s*=\s*")v\d+\.\d+\.\d+(")')
# every package of the page the app takes from a folder of appkit: appkit-ui (ui-kit) and appkit-web (web-kit)
UI = re.compile(r"(github:WKDSteve/appkit#)v\d+\.\d+\.\d+(&path:(?:ui-kit|web-kit))(?=\")")


def bump_workflow(text: str, tag: str, sha: str) -> tuple[str, int]:
    return USES.subn(lambda m: f"{m.group(1)}{sha}{m.group(2)}{tag}", text)


def bump_pin(text: str, tag: str, sha: str) -> tuple[str, int]:
    text, n_tag = PIN_TAG.subn(lambda m: f"{m.group(1)}{tag}", text)
    text, n_commit = PIN_COMMIT.subn(lambda m: f"{m.group(1)}{sha}", text)
    return text, min(n_tag, n_commit)


def bump_cargo(text: str, tag: str, _sha: str) -> tuple[str, int]:
    return CARGO.subn(lambda m: f"{m.group(1)}{tag}{m.group(2)}", text)


def bump_package_json(text: str, tag: str, _sha: str) -> tuple[str, int]:
    return UI.subn(lambda m: f"{m.group(1)}{tag}{m.group(2)}", text)


def read_exact(path: Path) -> str:
    """The text with its line endings as they are: read_text would turn CRLF into LF, and writing it back
    would change every line of the file."""
    with open(path, encoding="utf-8", newline="") as handle:
        return handle.read()


def plan(app: Path, mode: str, tag: str, sha: str) -> tuple[dict[Path, str], list[str]]:
    """The new text of every file that changes, and what is wrong (a place that should hold a pin and does not)."""
    changes: dict[Path, str] = {}
    problems: list[str] = []

    def apply(path: Path, fn, required: bool) -> None:
        if not path.is_file():
            if required:
                problems.append(f"{path}: not there")
            return
        old = read_exact(path)
        new, count = fn(old, tag, sha)
        if count == 0 and required:
            problems.append(f"{path}: no pin of appkit found in it")
        if new != old:
            changes[path] = new

    workflows = sorted((app / ".github" / "workflows").glob("*.y*ml"))
    for wf in workflows:
        apply(wf, bump_workflow, required=False)
    if not any(USES.search(read_exact(wf)) for wf in workflows):
        problems.append(f"{app}: no workflow takes an action of the release-kit")
    apply(app / "tools" / "release-kit.pin", bump_pin, required=(mode == "kit"))
    if mode == "full":
        apply(app / "Cargo.toml", bump_cargo, required=True)
        apply(app / "ui" / "package.json", bump_package_json, required=True)
    return changes, problems


def git(repo: Path, *args: str) -> str:
    res = subprocess.run(["git", "-C", str(repo), *args], capture_output=True, text=True, encoding="utf-8",
                         errors="replace")
    if res.returncode != 0:
        raise RuntimeError(f"git {' '.join(args)}: {res.stderr.strip() or res.stdout.strip()}")
    return res.stdout


def dirty_files(repo: Path, paths: list[Path]) -> list[str]:
    """Which of these paths have uncommitted changes (modified, staged or untracked)."""
    if not paths:
        return []
    rel = [os.path.relpath(p, repo).replace("\\", "/") for p in paths]
    out = git(repo, "status", "--porcelain", "--", *rel)
    return [line[3:].strip() for line in out.splitlines() if line.strip()]


def resolve_tag(appkit: Path, tag: str) -> str:
    """The commit of the tag, the same here and on origin; raises when it is not on both."""
    local = git(appkit, "rev-parse", "--verify", f"{tag}^{{commit}}").strip()
    remote = git(appkit, "ls-remote", "origin", f"refs/tags/{tag}", f"refs/tags/{tag}^{{}}")
    peeled = plain = ""
    for line in remote.splitlines():
        sha, _, ref = line.partition("\t")
        if ref.endswith("^{}"):
            peeled = sha
        else:
            plain = sha
    on_origin = peeled or plain
    if not on_origin:
        raise RuntimeError(f"{tag} is not on origin: push it first (git push origin {tag})")
    if on_origin != local:
        raise RuntimeError(f"{tag} names {local[:12]} here and {on_origin[:12]} on origin")
    return local


def run(cmd: list[str], cwd: Path, env: dict[str, str] | None = None, secret: str = "") -> None:
    """Run a command; on failure raise with the last lines of its output, `secret` (a token) replaced by ***."""
    print(f"    $ {' '.join(cmd)}   (in {cwd.name})")
    res = subprocess.run(cmd, cwd=cwd, env=env, capture_output=True, text=True, encoding="utf-8", errors="replace")
    if res.returncode != 0:
        output = (res.stdout + res.stderr).strip()
        if secret:
            output = output.replace(secret, "***")
        raise RuntimeError("failed:\n      " + "\n      ".join(output.splitlines()[-12:]))


def refresh_locks(app: Path) -> None:
    """Cargo.lock and the pnpm lock follow the new tag. pnpm reads the private appkit from codeload.github.com:
    the token of the `gh` login goes into a file of this run only (never into the user's ~/.npmrc)."""
    run(["cargo", "update", "-p", "appkit-hub"], app)
    token = subprocess.run(["gh", "auth", "token"], capture_output=True, text=True).stdout.strip()
    if not token:
        raise RuntimeError("gh has no login (gh auth login): pnpm cannot read the private appkit")
    pnpm = shutil.which("pnpm")
    if not pnpm:
        raise RuntimeError("pnpm is not on PATH")
    with tempfile.TemporaryDirectory() as folder:
        rc = Path(folder) / "pnpm-auth.npmrc"
        rc.write_text(f"//codeload.github.com/:_authToken={token}\n", encoding="utf-8")
        run([pnpm, "install"], app / "ui", dict(os.environ, NPM_CONFIG_USERCONFIG=str(rc)), secret=token)


def bump_app(name: str, repos: Path, tag: str, sha: str, args: argparse.Namespace) -> bool:
    mode = APPS[name]
    app = repos / name
    print(f"\n== {name} ({mode})")
    if not (app / ".git").exists():
        print("   skipped: no clone at " + str(app))
        return False
    changes, problems = plan(app, mode, tag, sha)
    if problems:
        print("   skipped, nothing written:\n     " + "\n     ".join(problems))
        return False
    if not changes:
        print(f"   already at {tag}")
        return True
    targets = list(changes)
    if mode == "full":
        targets += [app / "Cargo.lock", app / "ui" / "pnpm-lock.yaml"]
    dirty = dirty_files(app, [p for p in targets if p.exists()])
    if dirty:
        print("   skipped: uncommitted changes in files this bump edits (not mine to sweep in):\n     " + "\n     ".join(dirty))
        return False
    for path in changes:
        print(f"   {'would edit' if args.dry_run else 'edit'}: {os.path.relpath(path, app)}")
    if args.dry_run:
        return True
    for path, text in changes.items():
        path.write_text(text, encoding="utf-8", newline="")
    try:
        if mode == "full" and not args.no_lock:
            refresh_locks(app)
        check = "check-appkit-pin.py" if mode == "full" else "check-release-kit-pin.py"
        run([sys.executable, str(Path("tools") / check)], app)
    except RuntimeError as err:
        print(f"   FAILED: {err}\n   (files are edited, not committed: git -C {app} diff / git checkout -- . to undo)")
        return False
    print(f"   ok: pins agree on {tag}")
    if args.commit:
        files = [os.path.relpath(p, app) for p in targets if p.exists()]
        git(app, "add", "--", *files)
        # with the paths: only these files, whatever else the person has staged in the app stays staged
        git(app, "commit", "-m", f"chore(appkit): {tag} (release-kit actions, pins and lock files together; tools/bump-apps.py of appkit)",
            "--", *files)
        print("   committed (not pushed)")
    return True


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Move the apps to one tag of appkit.")
    parser.add_argument("tag", help="vX.Y.Z, already pushed to origin")
    parser.add_argument("--apps", default=",".join(APPS), help="comma separated (default: all five)")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--commit", action="store_true", help="commit each app that was bumped (never pushes)")
    parser.add_argument("--no-lock", action="store_true", help="edit the files only: no cargo update / pnpm install")
    parser.add_argument("--repos-dir", help="folder with the clones (default: the parent of this clone)")
    args = parser.parse_args(argv)

    if not TAG.match(args.tag):
        print(f"error: '{args.tag}' is not vX.Y.Z", file=sys.stderr)
        return 2
    names = [n.strip() for n in args.apps.split(",") if n.strip()]
    unknown = [n for n in names if n not in APPS]
    if unknown:
        print(f"error: unknown app(s): {', '.join(unknown)} (known: {', '.join(APPS)})", file=sys.stderr)
        return 2
    appkit = Path(__file__).resolve().parent.parent
    repos = Path(args.repos_dir) if args.repos_dir else appkit.parent
    try:
        sha = resolve_tag(appkit, args.tag)
    except RuntimeError as err:
        print(f"error: {err}", file=sys.stderr)
        return 2
    print(f"{args.tag} = {sha}")
    results = {name: bump_app(name, repos, args.tag, sha, args) for name in names}
    bad = [n for n, ok in results.items() if not ok]
    print("\n" + ("all done" if not bad else f"NOT done: {', '.join(bad)}"))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
