"""Tests of bump-apps.py: every place an app pins appkit is moved, nothing else changes, a place that holds no
pin is reported, and an app with uncommitted work in the files to edit is left alone. GitHub is never asked:
the tag and its commit are given, and git runs in throw-away repositories.

    python -m unittest discover tools
"""

import importlib.util
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

SPEC = importlib.util.spec_from_file_location("bump_apps", Path(__file__).with_name("bump-apps.py"))
bump = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(bump)

OLD_SHA = "2d0f4a6921d11848bbb579e2760614e88723e42d"
NEW_SHA = "0123456789abcdef0123456789abcdef01234567"
WORKFLOW = (
    "jobs:\n  a:\n    steps:\n"
    f"      - uses: WKDSteve/appkit/release-kit/prepare@{OLD_SHA} # v0.23.1\n"
    f"      - uses: WKDSteve/appkit/release-kit/publish@{OLD_SHA} # v0.23.1\n"
    "      - uses: actions/checkout@d23441a48e516b6c34aea4fa41551a30e30af803 # v6.1.0 (node24)\n"
)
PIN = f"# comment with tag = v9.9.9 in it\ntag = v0.23.1\ncommit = {OLD_SHA}\n"
CARGO = (
    '[workspace.dependencies]\n'
    'appkit-core = { git = "https://github.com/WKDSteve/appkit", tag = "v0.23.1" }\n'
    'appkit-hub = { git = "https://github.com/WKDSteve/appkit", tag = "v0.23.1" }\n'
    'serde = { version = "1" }\n'
)
PACKAGE = (
    '{\n  "dependencies": {\n    "appkit-ui": "github:WKDSteve/appkit#v0.23.1&path:ui-kit",\n'
    '    "appkit-web": "github:WKDSteve/appkit#v0.23.1&path:web-kit",\n    "svelte": "^5.0.0"\n  }\n}\n'
)


def make_app(root: Path, name: str, full: bool) -> Path:
    app = root / name
    (app / ".github" / "workflows").mkdir(parents=True)
    (app / "tools").mkdir()
    (app / ".github" / "workflows" / "rolling.yml").write_text(WORKFLOW, encoding="utf-8", newline="")
    (app / "tools" / "release-kit.pin").write_text(PIN, encoding="utf-8", newline="")
    if full:
        (app / "ui").mkdir()
        (app / "Cargo.toml").write_text(CARGO, encoding="utf-8", newline="")
        (app / "ui" / "package.json").write_text(PACKAGE, encoding="utf-8", newline="")
    return app


class BumpTextTest(unittest.TestCase):
    def test_workflow_moves_only_the_release_kit_lines(self) -> None:
        new, n = bump.bump_workflow(WORKFLOW, "v0.23.2", NEW_SHA)
        self.assertEqual(n, 2)
        self.assertEqual(new.count(f"@{NEW_SHA} # v0.23.2"), 2)
        self.assertNotIn(OLD_SHA, new)
        self.assertIn("actions/checkout@d23441a48e516b6c34aea4fa41551a30e30af803 # v6.1.0 (node24)", new)

    def test_pin_file_moves_tag_and_commit_and_not_a_comment(self) -> None:
        new, n = bump.bump_pin(PIN, "v0.23.2", NEW_SHA)
        self.assertEqual(n, 1)
        self.assertIn("tag = v0.23.2\n", new)
        self.assertIn(f"commit = {NEW_SHA}\n", new)
        self.assertIn("# comment with tag = v9.9.9 in it", new)

    def test_cargo_and_package_json(self) -> None:
        cargo, n = bump.bump_cargo(CARGO, "v0.23.2", NEW_SHA)
        self.assertEqual(n, 2)
        self.assertEqual(cargo.count('tag = "v0.23.2"'), 2)
        self.assertIn('serde = { version = "1" }', cargo)
        package, m = bump.bump_package_json(PACKAGE, "v0.23.2", NEW_SHA)
        self.assertEqual(m, 2, "appkit-ui and appkit-web move together")
        self.assertIn("github:WKDSteve/appkit#v0.23.2&path:ui-kit", package)
        self.assertIn("github:WKDSteve/appkit#v0.23.2&path:web-kit", package)
        self.assertNotIn("v0.23.1", package)
        # Another folder of appkit is not a package this tool knows.
        _, k = bump.bump_package_json('"x": "github:WKDSteve/appkit#v0.23.1&path:ui-kit-old"', "v0.23.2", NEW_SHA)
        self.assertEqual(k, 0)
        self.assertIn('"svelte": "^5.0.0"', package)

    def test_a_second_run_changes_nothing(self) -> None:
        once, _ = bump.bump_workflow(WORKFLOW, "v0.23.2", NEW_SHA)
        twice, _ = bump.bump_workflow(once, "v0.23.2", NEW_SHA)
        self.assertEqual(once, twice)


class PlanTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)

    def test_a_kit_only_app_edits_workflows_and_pin(self) -> None:
        app = make_app(self.tmp, "ai-desktop", full=False)
        changes, problems = bump.plan(app, "kit", "v0.23.2", NEW_SHA)
        self.assertEqual(problems, [])
        self.assertEqual({p.name for p in changes}, {"rolling.yml", "release-kit.pin"})

    def test_a_full_app_also_edits_cargo_and_the_page(self) -> None:
        app = make_app(self.tmp, "tasks-desktop", full=True)
        changes, problems = bump.plan(app, "full", "v0.23.2", NEW_SHA)
        self.assertEqual(problems, [])
        self.assertEqual({p.name for p in changes}, {"rolling.yml", "release-kit.pin", "Cargo.toml", "package.json"})

    def test_a_place_without_a_pin_is_a_problem_not_a_silent_skip(self) -> None:
        app = make_app(self.tmp, "tasks-desktop", full=True)
        (app / "Cargo.toml").write_text("[workspace.dependencies]\nserde = \"1\"\n", encoding="utf-8")
        _, problems = bump.plan(app, "full", "v0.23.2", NEW_SHA)
        self.assertTrue(any("Cargo.toml" in p and "no pin" in p for p in problems), problems)

    def test_an_app_that_uses_no_action_of_the_kit_is_a_problem(self) -> None:
        app = make_app(self.tmp, "ai-desktop", full=False)
        (app / ".github" / "workflows" / "rolling.yml").write_text("jobs: {}\n", encoding="utf-8")
        _, problems = bump.plan(app, "kit", "v0.23.2", NEW_SHA)
        self.assertTrue(any("no workflow takes an action" in p for p in problems), problems)


class DirtyTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)

    def git(self, *args: str) -> None:
        subprocess.run(["git", "-C", str(self.tmp), "-c", "user.name=t", "-c", "user.email=t@t", *args],
                       check=True, capture_output=True)

    def test_uncommitted_work_in_a_file_to_edit_is_seen(self) -> None:
        self.git("init", "-q")
        (self.tmp / "a.txt").write_text("a\n")
        (self.tmp / "b.txt").write_text("b\n")
        self.git("add", ".")
        self.git("commit", "-q", "-m", "x")
        self.assertEqual(bump.dirty_files(self.tmp, [self.tmp / "a.txt", self.tmp / "b.txt"]), [])
        (self.tmp / "a.txt").write_text("changed\n")
        self.assertEqual(bump.dirty_files(self.tmp, [self.tmp / "a.txt", self.tmp / "b.txt"]), ["a.txt"])
        self.assertEqual(bump.dirty_files(self.tmp, [self.tmp / "b.txt"]), [])


class BumpAppTest(unittest.TestCase):
    """bump_app end to end in throw-away git repositories; the lock refresh and the pin check are replaced."""

    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        self.args = type("Args", (), {"dry_run": False, "commit": False, "no_lock": False})()

    def git(self, app: Path, *args: str) -> str:
        res = subprocess.run(["git", "-C", str(app), "-c", "user.name=t", "-c", "user.email=t@t", *args],
                             check=True, capture_output=True, text=True)
        return res.stdout

    def repo(self, name: str, full: bool) -> Path:
        app = make_app(self.tmp, name, full)
        self.git(app, "init", "-q")
        self.git(app, "add", ".")
        self.git(app, "commit", "-q", "-m", "start")
        return app

    def test_a_kit_app_is_bumped_checked_and_committed(self) -> None:
        app = self.repo("ai-desktop", full=False)
        self.args.commit = True
        with mock.patch.object(bump, "run") as run:
            self.assertTrue(bump.bump_app("ai-desktop", self.tmp, "v0.23.2", NEW_SHA, self.args))
        self.assertIn(f"@{NEW_SHA} # v0.23.2", (app / ".github" / "workflows" / "rolling.yml").read_text())
        self.assertEqual(self.git(app, "status", "--porcelain").strip(), "")
        self.assertEqual(len(self.git(app, "log", "--oneline").splitlines()), 2)
        self.assertEqual(run.call_args.args[0][-1], str(Path("tools") / "check-release-kit-pin.py"))

    def test_a_full_app_refreshes_the_locks_and_runs_its_own_check(self) -> None:
        app = self.repo("tasks-desktop", full=True)
        with mock.patch.object(bump, "run") as run, mock.patch.object(bump, "refresh_locks") as refresh:
            self.assertTrue(bump.bump_app("tasks-desktop", self.tmp, "v0.23.2", NEW_SHA, self.args))
        refresh.assert_called_once_with(app)
        self.assertEqual(run.call_args.args[0][-1], str(Path("tools") / "check-appkit-pin.py"))
        self.assertIn('tag = "v0.23.2"', (app / "Cargo.toml").read_text())

    def test_a_failing_lock_refresh_is_a_failure_and_leaves_the_edits_uncommitted(self) -> None:
        app = self.repo("tasks-desktop", full=True)
        self.args.commit = True
        with mock.patch.object(bump, "run"), mock.patch.object(bump, "refresh_locks", side_effect=RuntimeError("no network")):
            self.assertFalse(bump.bump_app("tasks-desktop", self.tmp, "v0.23.2", NEW_SHA, self.args))
        self.assertEqual(len(self.git(app, "log", "--oneline").splitlines()), 1, "nothing was committed")
        self.assertIn("Cargo.toml", self.git(app, "status", "--porcelain"))

    def test_a_failing_pin_check_is_a_failure_and_commits_nothing(self) -> None:
        app = self.repo("ai-desktop", full=False)
        self.args.commit = True
        with mock.patch.object(bump, "run", side_effect=RuntimeError("pins differ")):
            self.assertFalse(bump.bump_app("ai-desktop", self.tmp, "v0.23.2", NEW_SHA, self.args))
        self.assertEqual(len(self.git(app, "log", "--oneline").splitlines()), 1)

    def test_uncommitted_work_in_a_file_to_edit_skips_the_app_untouched(self) -> None:
        app = self.repo("ai-desktop", full=False)
        mine = app / ".github" / "workflows" / "rolling.yml"
        mine.write_text(WORKFLOW + "# someone else's work\n", encoding="utf-8", newline="")
        with mock.patch.object(bump, "run") as run:
            self.assertFalse(bump.bump_app("ai-desktop", self.tmp, "v0.23.2", NEW_SHA, self.args))
        run.assert_not_called()
        self.assertEqual(mine.read_text(), WORKFLOW + "# someone else's work\n", "the file was not touched")
        self.assertIn(OLD_SHA, (app / "tools" / "release-kit.pin").read_text(), "no other file was touched either")

    def test_dry_run_writes_nothing(self) -> None:
        app = self.repo("ai-desktop", full=False)
        self.args.dry_run = True
        self.assertTrue(bump.bump_app("ai-desktop", self.tmp, "v0.23.2", NEW_SHA, self.args))
        self.assertEqual(self.git(app, "status", "--porcelain").strip(), "")

    def test_line_endings_of_the_files_stay_as_they_are(self) -> None:
        # A CRLF file must not be rewritten as LF (a diff of every line) and its pin lines must still be found.
        app = self.repo("ai-desktop", full=False)
        for path in (app / ".github" / "workflows" / "rolling.yml", app / "tools" / "release-kit.pin"):
            path.write_bytes(path.read_bytes().replace(b"\n", b"\r\n"))
        self.git(app, "-c", "core.autocrlf=false", "commit", "-q", "-am", "crlf")
        with mock.patch.object(bump, "run"):
            self.assertTrue(bump.bump_app("ai-desktop", self.tmp, "v0.23.2", NEW_SHA, self.args))
        for path in (app / ".github" / "workflows" / "rolling.yml", app / "tools" / "release-kit.pin"):
            data = path.read_bytes()
            self.assertIn(NEW_SHA.encode(), data)
            self.assertEqual(data.count(b"\n"), data.count(b"\r\n"), f"{path.name} lost its CRLF")
        diff = self.git(app, "-c", "core.autocrlf=false", "diff", "--numstat")
        self.assertEqual(sum(int(line.split()[0]) for line in diff.splitlines()), 4, diff)  # 2 + 2 lines, not whole files

    def test_a_commit_takes_only_the_files_of_the_bump(self) -> None:
        # Something the person had staged in the app is not swept into the bump commit.
        app = self.repo("ai-desktop", full=False)
        (app / "other.txt").write_text("not mine\n")
        self.git(app, "add", "other.txt")
        self.args.commit = True
        with mock.patch.object(bump, "run"):
            self.assertTrue(bump.bump_app("ai-desktop", self.tmp, "v0.23.2", NEW_SHA, self.args))
        self.assertNotIn("other.txt", self.git(app, "show", "--name-only", "--format=", "HEAD"))
        self.assertIn("A  other.txt", self.git(app, "status", "--porcelain"))

    def test_the_token_is_not_printed_when_a_command_fails(self) -> None:
        with self.assertRaises(RuntimeError) as caught:
            bump.run([sys.executable, "-c", "import sys; print('//codeload.github.com/:_authToken=s3cret-token'); sys.exit(1)"],
                     self.tmp, secret="s3cret-token")
        self.assertNotIn("s3cret-token", str(caught.exception))
        self.assertIn("***", str(caught.exception))

    def test_an_app_that_is_already_there_is_ok(self) -> None:
        self.repo("ai-desktop", full=False)
        with mock.patch.object(bump, "run"):
            self.assertTrue(bump.bump_app("ai-desktop", self.tmp, "v0.23.2", NEW_SHA, self.args))
            self.git(self.tmp / "ai-desktop", "commit", "-q", "-am", "bumped")
            self.assertTrue(bump.bump_app("ai-desktop", self.tmp, "v0.23.2", NEW_SHA, self.args))


class TagTest(unittest.TestCase):
    def test_only_vxyz_is_a_tag(self) -> None:
        for good in ("v0.23.2", "v1.0.0"):
            self.assertTrue(bump.TAG.match(good), good)
        for bad in ("0.23.2", "v0.23", "v0.23.2-rc1", "main", "v0.23.2 "):
            self.assertFalse(bump.TAG.match(bad), bad)

    def test_hub_desktop_is_a_full_app(self) -> None:
        self.assertEqual(bump.APPS["hub-desktop"], "full")

    def test_the_cli_refuses_a_bad_tag_and_an_unknown_app(self) -> None:
        self.assertEqual(bump.main(["main"]), 2)
        self.assertEqual(bump.main(["v0.23.2", "--apps", "nope"]), 2)


if __name__ == "__main__":
    unittest.main(argv=[sys.argv[0]])
