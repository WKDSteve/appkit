#!/usr/bin/env python3
"""Move the apps that use appkit to one tag of appkit, in one command.

    python tools/bump-apps.py v0.23.2                       all seven apps, files edited, lock files refreshed, pins checked
    python tools/bump-apps.py v0.23.2 --apps ai-desktop     only these (comma separated)
    python tools/bump-apps.py v0.23.2 --dry-run             say what would change, change nothing
    python tools/bump-apps.py v0.23.2 --commit              also commit each app (never pushes)
    python tools/bump-apps.py v0.23.2 --hold-back           a crate the bump made too young goes back to appkit's version
    python tools/bump-apps.py v0.23.2 --worktree            bump and commit each app in a worktree of its HEAD (see below)

Why a script: an app pins appkit in several places that must agree (and its own check fails when they do
not), and doing it by hand is what makes a small appkit fix cost an afternoon.

  kit only (ai-desktop, docmap-desktop)   the `uses: WKDSteve/appkit/release-kit/...@<sha> # vX.Y.Z` lines of the
                                          workflows and tools/release-kit.pin
  full (tasks-desktop, rules-desktop, testing-desktop, hub-desktop)
                                          the same, and the tag in Cargo.toml and ui/package.json (appkit-ui and appkit-web); then
                                          `cargo update -p appkit-hub` and `pnpm install` rewrite the lock files
  cargo (pult)                            the workflows and the tag in Cargo.toml, no page and no release-kit.pin;
                                          `cargo update -p appkit-hub` rewrites Cargo.lock

The tag must exist in this clone AND on origin, and name the same commit in both: a pin to a commit that GitHub
does not know turns every build red. An app whose files to edit have uncommitted changes of their own is
skipped (another session's work is never swept into a bump commit). When done, each app's own pin check runs
(tools/check-appkit-pin.py or tools/check-release-kit-pin.py); the exit code is 1 when one of them fails.
A full app that has tools/check-lock-age.py runs it after the lock refresh (`--min-days`, default 7, as its CI): a
`cargo update -p appkit-hub` can pull a dependency of the kit in at a version that is days old, and the app's CI
would turn red on it. Such a crate is named and the app is not done (nothing committed). With `--hold-back` it is
put back with `cargo update -p <name>@<version> --precise <v>` to the version appkit's own Cargo.lock holds (which
passed the same gate here), and the check runs again; a crate appkit does not hold, or holds only at the same
version, stays a failure. The check asks crates.io; `--no-lock` skips it with the refresh.

--worktree (implies --commit): the bump of each app is made in a throw-away git worktree of the app's HEAD, never in
the clone. Uncommitted work of another session then neither skips the app nor slips into the lock files (`cargo
update` resolves the whole workspace, so a dependency added to a member's Cargo.toml and not yet committed would land
in the bump's Cargo.lock; pnpm runs with --lockfile-only there). The commit is moved into the clone's branch as a
fast-forward; when the clone's own uncommitted changes touch the same files, or its branch moved meanwhile, the commit
stays on the branch bump/appkit-<tag> and the app counts as not done (merge it once that work is committed).

A job whose step `release-kit/publish` attests (the default since v0.48) needs `id-token: write` and
`attestations: write` in its permissions (or the workflow's, when the job has none): without them every rolling and
release run fails, and nothing in the app's CI notices. Such a job is a problem: the app is skipped, nothing written,
until its workflow grants them (or the step says `attest: false`).

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
    "testing-desktop": "full",
    "hub-desktop": "full",
    "pult": "cargo",
}

TAG = re.compile(r"^v\d+\.\d+\.\d+$")
USES = re.compile(r"(uses:\s*WKDSteve/appkit/release-kit/[A-Za-z0-9_./-]+@)[0-9a-f]{40}([ \t]*#[ \t]*)v\d+\.\d+\.\d+")
# (?=[ \t]*\r?$): the line may end in CRLF, and the end of the line is not part of what is replaced
PIN_TAG = re.compile(r"^(tag\s*=\s*)v\d+\.\d+\.\d+(?=[ \t]*\r?$)", re.M)
PIN_COMMIT = re.compile(r"^(commit\s*=\s*)[0-9a-f]{40}(?=[ \t]*\r?$)", re.M)
CARGO = re.compile(r'(git\s*=\s*"https://github\.com/WKDSteve/appkit",\s*tag\s*=\s*")v\d+\.\d+\.\d+(")')
# every package of the page the app takes from a folder of appkit: appkit-ui (ui-kit) and appkit-web (web-kit)
# one line of check-lock-age.py: "  too young: ctutils 0.4.8 (5.9 days)"
YOUNG = re.compile(r"^\s*too young: (\S+) (\S+) \(", re.M)
APPKIT = Path(__file__).resolve().parent.parent
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


# Cargo projects of their own inside an app (their own Cargo.toml and Cargo.lock), moved with the app: testing-desktop's
# `mobile/` pins appkit-mobile and tauri-plugin-appkit. A missing one is no problem.
NESTED = ("mobile",)


def nested_pinned(app: Path) -> list[str]:
    """The nested projects of `app` whose Cargo.toml pins appkit."""
    found = []
    for sub in NESTED:
        manifest = app / sub / "Cargo.toml"
        if manifest.is_file() and "github.com/WKDSteve/appkit" in read_exact(manifest):
            found.append(sub)
    return found


TOP_JOBS = re.compile(r"^jobs:\s*(?:#.*)?$")
JOB_KEY = re.compile(r"^(\s+)([A-Za-z0-9_-]+):\s*(?:#.*)?$")
PERMISSIONS = re.compile(r"^(\s*)permissions:\s*([^#]*?)\s*(?:#.*)?$")
ATTEST_OFF = re.compile(r"""^\s+attest:\s*['"]?false['"]?\s*(?:#.*)?$""")
ATTEST_NEEDS = ("id-token", "attestations")


def indent(line: str) -> int:
    return len(line) - len(line.lstrip(" "))


def granted(block: list[str]) -> set[str] | None:
    """The scopes a `permissions:` block grants `write` (all of them for write-all); None when there is no block.
    `block` is the lines of the job (or of the top of the workflow) the block may sit in, at its own indentation."""
    if not block:
        return None
    level = min((indent(line) for line in block if line.strip() and not line.lstrip().startswith("#")), default=0)
    for i, line in enumerate(block):
        m = PERMISSIONS.match(line)
        if not m or len(m.group(1)) != level:
            continue
        value = m.group(2)
        if value == "write-all":
            return set(ATTEST_NEEDS)
        if value.startswith("{"):
            pairs = (part.split(":", 1) for part in value.strip("{}").split(",") if ":" in part)
            return {k.strip() for k, v in pairs if v.strip() == "write"}
        if value:
            return set()  # read-all, or anything else that grants no write
        scopes = set()
        for inner in block[i + 1:]:
            if inner.strip() and indent(inner) <= level:
                break
            key, _, val = inner.strip().partition(":")
            if val.split("#", 1)[0].strip() == "write":
                scopes.add(key.strip())
        return scopes
    return None


def jobs_that_cannot_attest(text: str) -> list[str]:
    """The jobs of a workflow whose step `release-kit/publish` attests (since appkit v0.48, unless `attest: false`)
    but whose token may not: without `id-token: write` and `attestations: write` (of the job, else of the workflow)
    publish stops with "this job has no OIDC token" and nothing is put on the release."""
    lines = text.splitlines()
    start = next((i for i, line in enumerate(lines) if TOP_JOBS.match(line)), None)
    if start is None:
        return []
    top = lines[:start]
    jobs: list[tuple[str, list[str]]] = []
    job_level: int | None = None
    for line in lines[start + 1:]:
        if line.strip() and indent(line) == 0 and not line.startswith("#"):
            break  # the next key of the top
        m = JOB_KEY.match(line)
        if m and (job_level is None or len(m.group(1)) == job_level):
            job_level = len(m.group(1))
            jobs.append((m.group(2), []))
        elif jobs:
            jobs[-1][1].append(line)
    missing = []
    for name, body in jobs:
        if not any("release-kit/publish@" in line for line in body):
            continue
        if any(ATTEST_OFF.match(line) for line in body):
            continue
        scopes = granted(body)
        if scopes is None:
            scopes = granted(top) or set()
        if not set(ATTEST_NEEDS) <= scopes:
            missing.append(name)
    return missing


def appkit_packages(lock_text: str) -> list[str]:
    """The names of the packages a Cargo.lock takes from appkit's repository, in the order of the lock."""
    names: list[str] = []
    for block in lock_text.split("[[package]]"):
        name = next((line.split("=", 1)[1].strip().strip('"') for line in block.splitlines()
                     if line.startswith("name = ")), None)
        if name and "github.com/WKDSteve/appkit" in block and name not in names:
            names.append(name)
    return names


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
    for wf in workflows:
        for job in jobs_that_cannot_attest(changes.get(wf, read_exact(wf))):
            problems.append(
                f"{wf}: job '{job}' publishes with the release-kit, which attests, but may not: give the job "
                "'id-token: write' and 'attestations: write' (or set attest: false on the step)"
            )
    apply(app / "tools" / "release-kit.pin", bump_pin, required=(mode == "kit"))
    if mode in ("full", "cargo"):
        apply(app / "Cargo.toml", bump_cargo, required=True)
        for sub in NESTED:
            apply(app / sub / "Cargo.toml", bump_cargo, required=False)
    if mode == "full":
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


def refresh_locks(app: Path, page: bool = True, lockfile_only: bool = False) -> None:
    """Cargo.lock and the pnpm lock follow the new tag; an app without a page (`page=False`) has only Cargo.lock.
    pnpm reads the private appkit from codeload.github.com: the token of the `gh` login goes into a file of this run
    only (never into the user's ~/.npmrc). `lockfile_only`: pnpm writes the lock and installs nothing (a fresh
    worktree has no node_modules)."""
    run(["cargo", "update", "-p", "appkit-hub"], app)
    for sub in nested_pinned(app):
        lock = app / sub / "Cargo.lock"
        names = appkit_packages(read_exact(lock)) if lock.is_file() else []
        if names:
            update = ["cargo", "update", "--manifest-path", str(Path(sub) / "Cargo.toml")]
            for name in names:
                update += ["-p", name]
            run(update, app)
    if not page:
        return
    token = subprocess.run(["gh", "auth", "token"], capture_output=True, text=True).stdout.strip()
    if not token:
        raise RuntimeError("gh has no login (gh auth login): pnpm cannot read the private appkit")
    pnpm = shutil.which("pnpm")
    if not pnpm:
        raise RuntimeError("pnpm is not on PATH")
    with tempfile.TemporaryDirectory() as folder:
        rc = Path(folder) / "pnpm-auth.npmrc"
        rc.write_text(f"//codeload.github.com/:_authToken={token}\n", encoding="utf-8")
        cmd = [pnpm, "install", "--lockfile-only"] if lockfile_only else [pnpm, "install"]
        run(cmd, app / "ui", dict(os.environ, NPM_CONFIG_USERCONFIG=str(rc)), secret=token)


def young_crates(output: str) -> list[tuple[str, str]]:
    """(name, version) of every crate check-lock-age.py calls too young."""
    return YOUNG.findall(output)


def registry_versions(lock_text: str, name: str) -> list[str]:
    """The versions of a crate from a registry in a Cargo.lock, read the way check-lock-age.py reads them."""
    found = []
    for block in lock_text.split("[[package]]")[1:]:
        n = re.search(r'^name = "([^"]+)"', block, re.M)
        v = re.search(r'^version = "([^"]+)"', block, re.M)
        if n and v and n.group(1) == name and re.search(r'^source = "registry\+', block, re.M):
            found.append(v.group(1))
    return found


def hold_back_version(appkit_lock: str, name: str, young: str) -> str | None:
    """The version to go back to: the one appkit's lock holds, when it holds exactly one other than the young one."""
    versions = [v for v in registry_versions(appkit_lock, name) if v != young]
    return versions[0] if len(versions) == 1 else None


def check_lock_age(app: Path, min_days: int) -> list[tuple[str, str]] | None:
    """The crates of the app's lock that are too young, by the app's own check; None when the app has no such check.
    Raises when the check itself fails (crates.io not reachable: exit 2)."""
    script = Path("tools") / "check-lock-age.py"
    if not (app / script).is_file():
        return None
    cmd = [sys.executable, str(script), "--min-days", str(min_days)]
    print(f"    $ {' '.join(cmd)}   (in {app.name})")
    res = subprocess.run(cmd, cwd=app, capture_output=True, text=True, encoding="utf-8", errors="replace")
    young = young_crates(res.stdout)
    if res.returncode == 0 and not young:
        return []
    if res.returncode == 1 and young:
        return young
    output = (res.stdout + res.stderr).strip()
    raise RuntimeError("the lock-age check failed:\n      " + "\n      ".join(output.splitlines()[-12:]))


def settle_lock_age(app: Path, min_days: int, hold_back: bool, appkit_lock: Path) -> None:
    """Raise when the bump left a crate in the lock that is younger than the app's CI takes; with hold_back, first
    put each such crate back to the version of appkit's lock, then check again."""
    young = check_lock_age(app, min_days)
    if not young:
        return
    if hold_back:
        lock = read_exact(appkit_lock) if appkit_lock.is_file() else ""
        for name, version in young:
            target = hold_back_version(lock, name, version)
            if target is None:
                continue
            run(["cargo", "update", "-p", f"{name}@{version}", "--precise", target], app)
            print(f"   held back: {name} {version} -> {target} (the version of appkit's Cargo.lock)")
        young = check_lock_age(app, min_days) or []
        if not young:
            return
    names = ", ".join(f"{n} {v}" for n, v in young)
    hint = "" if hold_back else " (--hold-back puts them back to the version of appkit's Cargo.lock)"
    raise RuntimeError(f"younger than {min_days} days in Cargo.lock: {names}{hint}")


def lock_targets(app: Path, mode: str, changes: dict[Path, str]) -> list[Path]:
    """The files a bump writes: the edited ones and the lock files the refresh rewrites."""
    targets = list(changes)
    if mode in ("full", "cargo"):
        targets.append(app / "Cargo.lock")
        for sub in NESTED:
            if app / sub / "Cargo.toml" in changes:
                targets.append(app / sub / "Cargo.lock")
    if mode == "full":
        targets.append(app / "ui" / "pnpm-lock.yaml")
    return targets


def check_nested_pins(app: Path, changes: dict[Path, str]) -> None:
    """The app's own pin check against the lock of each nested project the bump moved."""
    for sub in NESTED:
        if app / sub / "Cargo.toml" in changes and (app / sub / "Cargo.lock").is_file():
            run([sys.executable, str(Path("tools") / "check-appkit-pin.py"), "--lock",
                 str(Path(sub) / "Cargo.lock")], app)


def bump_in_worktree(name: str, app: Path, mode: str, tag: str, sha: str, args: argparse.Namespace) -> bool:
    """The bump of one app in a throw-away worktree of its HEAD, committed there and moved into the clone's branch
    as a fast-forward (or left on bump/appkit-<tag>)."""
    try:
        branch = git(app, "symbolic-ref", "--quiet", "--short", "HEAD").strip()
    except RuntimeError:
        print("   skipped: the clone is on no branch (a detached HEAD): nothing to move the bump to")
        return False
    base = git(app, "rev-parse", "HEAD").strip()
    holder = Path(tempfile.mkdtemp(prefix=f"bump-{name}-"))
    tree = holder / name
    try:
        git(app, "worktree", "add", "--detach", str(tree), base)
        changes, problems = plan(tree, mode, tag, sha)
        if problems:
            print("   skipped, nothing written:\n     " + "\n     ".join(problems))
            return False
        if not changes:
            print(f"   already at {tag}")
            return True
        for path in changes:
            verb = "would edit" if args.dry_run else "edit"
            print(f"   {verb}: {os.path.relpath(path, tree)}   (in a worktree of {base[:12]})")
        if args.dry_run:
            return True
        for path, text in changes.items():
            path.write_text(text, encoding="utf-8", newline="")
        try:
            crates = mode in ("full", "cargo")
            if crates and not args.no_lock:
                refresh_locks(tree, page=(mode == "full"), lockfile_only=True)
            check = "check-appkit-pin.py" if crates else "check-release-kit-pin.py"
            run([sys.executable, str(Path("tools") / check)], tree)
            if crates and not args.no_lock:
                check_nested_pins(tree, changes)
            if crates and not args.no_lock:
                settle_lock_age(tree, getattr(args, "min_days", 7), getattr(args, "hold_back", False),
                                Path(getattr(args, "appkit_lock", APPKIT / "Cargo.lock")))
        except RuntimeError as err:
            print(f"   FAILED: {err}\n   (nothing committed; the worktree is removed)")
            return False
        files = [os.path.relpath(p, tree) for p in lock_targets(tree, mode, changes) if p.exists()]
        git(tree, "add", "--", *files)
        git(tree, "commit", "-m",
            f"chore(appkit): {tag} (release-kit actions, pins and lock files together; tools/bump-apps.py of appkit)",
            "--", *files)
        commit = git(tree, "rev-parse", "HEAD").strip()
        print(f"   ok: pins agree on {tag}, committed as {commit[:12]} in the worktree")
        try:
            # a fast-forward of the clone's branch; git refuses when the clone's own changes touch the same files
            git(app, "merge", "--ff-only", "--quiet", commit)
        except RuntimeError as err:
            keep = f"bump/appkit-{tag}"
            git(app, "branch", "--force", keep, commit)
            print(f"   NOT moved into {branch}: {err}\n   the commit stays on {keep}: "
                  f"git -C {app} merge --ff-only {keep} once the clone's own work is committed")
            return False
        print(f"   moved into {branch} (fast-forward; not pushed)")
        return True
    finally:
        try:
            git(app, "worktree", "remove", "--force", str(tree))
        except RuntimeError:
            pass
        shutil.rmtree(holder, ignore_errors=True)
        try:
            git(app, "worktree", "prune")
        except RuntimeError:
            pass


def bump_app(name: str, repos: Path, tag: str, sha: str, args: argparse.Namespace) -> bool:
    mode = APPS[name]
    app = repos / name
    print(f"\n== {name} ({mode})")
    if not (app / ".git").exists():
        print("   skipped: no clone at " + str(app))
        return False
    if getattr(args, "worktree", False):
        return bump_in_worktree(name, app, mode, tag, sha, args)
    changes, problems = plan(app, mode, tag, sha)
    if problems:
        print("   skipped, nothing written:\n     " + "\n     ".join(problems))
        return False
    if not changes:
        print(f"   already at {tag}")
        return True
    targets = lock_targets(app, mode, changes)
    dirty = dirty_files(app, [p for p in targets if p.exists()])
    if dirty:
        print("   skipped: uncommitted changes in files this bump edits (not mine to sweep in; --worktree bumps"
              " from HEAD):\n     " + "\n     ".join(dirty))
        return False
    for path in changes:
        print(f"   {'would edit' if args.dry_run else 'edit'}: {os.path.relpath(path, app)}")
    if args.dry_run:
        return True
    for path, text in changes.items():
        path.write_text(text, encoding="utf-8", newline="")
    try:
        crates = mode in ("full", "cargo")
        if crates and not args.no_lock:
            refresh_locks(app, page=(mode == "full"))
        check = "check-appkit-pin.py" if crates else "check-release-kit-pin.py"
        run([sys.executable, str(Path("tools") / check)], app)
        if crates and not args.no_lock:
            check_nested_pins(app, changes)
            settle_lock_age(app, getattr(args, "min_days", 7), getattr(args, "hold_back", False),
                            Path(getattr(args, "appkit_lock", APPKIT / "Cargo.lock")))
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
    parser.add_argument("--apps", default=",".join(APPS), help="comma separated (default: all of them)")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--commit", action="store_true", help="commit each app that was bumped (never pushes)")
    parser.add_argument("--no-lock", action="store_true", help="edit the files only: no cargo update / pnpm install")
    parser.add_argument("--min-days", type=int, default=7, help="the minimum age of a crate, as the apps' CI (default 7)")
    parser.add_argument("--hold-back", action="store_true",
                        help="put a crate the bump made too young back to the version of appkit's Cargo.lock")
    parser.add_argument("--worktree", action="store_true",
                        help="bump and commit each app in a worktree of its HEAD, then fast-forward the clone"
                             " (implies --commit)")
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
    appkit = APPKIT
    args.appkit_lock = appkit / "Cargo.lock"
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
