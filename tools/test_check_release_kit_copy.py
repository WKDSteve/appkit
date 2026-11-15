"""Tests of check-release-kit-copy.py against a git repository made here (no network)."""

from __future__ import annotations

import importlib.util
import io
import shutil
import subprocess
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("check_release_kit_copy", HERE / "check-release-kit-copy.py")
assert spec and spec.loader
tool = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tool)

KIT_PATH = "release-kit/scripts/uninstall-entry.ps1"


def git(cwd: Path, *args: str) -> str:
    return subprocess.run(["git", "-C", str(cwd), *args], check=True, capture_output=True, text=True).stdout.strip()


class CheckReleaseKitCopyTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        # A stand-in for appkit with the file in two versions.
        self.kit = self.tmp / "appkit"
        (self.kit / "release-kit" / "scripts").mkdir(parents=True)
        git(self.kit, "init", "-q")
        git(self.kit, "config", "user.email", "t@example.invalid")
        git(self.kit, "config", "user.name", "t")
        git(self.kit, "config", "commit.gpgsign", "false")
        (self.kit / KIT_PATH).write_bytes(b"# one\nfunction A {}\n")
        git(self.kit, "add", ".")
        git(self.kit, "commit", "-q", "-m", "one")
        self.old = git(self.kit, "rev-parse", "HEAD")
        (self.kit / KIT_PATH).write_bytes(b"# two\nfunction A {}\nfunction B {}\n")
        git(self.kit, "commit", "-q", "-am", "two")
        self.new = git(self.kit, "rev-parse", "HEAD")
        # The app: workflows that pin the release kit, and its copy.
        self.app = self.tmp / "app"
        (self.app / ".github" / "workflows").mkdir(parents=True)
        (self.app / "scripts").mkdir()

    def pin(self, name: str, sha: str) -> None:
        (self.app / ".github" / "workflows" / name).write_text(
            f"jobs:\n  a:\n    steps:\n      - uses: WKDSteve/appkit/release-kit/publish@{sha} # vX\n", encoding="utf-8")

    def run_tool(self) -> tuple[int, str]:
        out = io.StringIO()
        with redirect_stdout(out), redirect_stderr(out):
            code = tool.main(["--root", str(self.app), "--appkit-dir", str(self.kit)])
        return code, out.getvalue()

    def test_the_copy_of_the_pinned_commit_passes_and_line_ends_do_not_count(self) -> None:
        self.pin("rolling.yml", self.new)
        self.pin("release.yml", self.new)
        (self.app / "scripts" / "uninstall-entry.ps1").write_bytes(b"# two\r\nfunction A {}\r\nfunction B {}\r\n")
        self.assertEqual(self.run_tool(), (0, "check-release-kit-copy: every copy is the file of the pinned release kit\n"))

    def test_a_copy_of_another_commit_or_none_fails_and_says_what_to_take(self) -> None:
        self.pin("rolling.yml", self.new)
        (self.app / "scripts" / "uninstall-entry.ps1").write_bytes(b"# one\nfunction A {}\n")
        code, said = self.run_tool()
        self.assertEqual(code, 1)
        self.assertIn(f"git -C <appkit> show {self.new}:{KIT_PATH}", said)
        (self.app / "scripts" / "uninstall-entry.ps1").unlink()
        code, said = self.run_tool()
        self.assertEqual(code, 1)
        self.assertIn("is missing", said)

    def test_pins_that_disagree_fail_and_no_pin_is_not_a_pass(self) -> None:
        self.pin("rolling.yml", self.new)
        self.pin("release.yml", self.old)
        code, said = self.run_tool()
        self.assertEqual(code, 1)
        self.assertIn("more than one commit", said)
        for path in (self.app / ".github" / "workflows").iterdir():
            path.unlink()
        self.assertEqual(self.run_tool()[0], 2)

    def test_a_kit_without_the_file_at_the_pin_is_not_a_pass(self) -> None:
        self.pin("rolling.yml", "0" * 40)
        self.assertEqual(self.run_tool()[0], 2)


if __name__ == "__main__":
    unittest.main()
