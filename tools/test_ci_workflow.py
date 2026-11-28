"""Tests of the CI workflow itself: `python -m unittest discover tools`."""

import pathlib
import re
import unittest

WORKFLOW = pathlib.Path(__file__).resolve().parent.parent / ".github" / "workflows" / "ci.yml"


def steps(text):
    """The `- ` steps of the workflow as blocks of lines, each with its indentation."""
    blocks, current, indent = [], [], None
    for line in text.splitlines():
        match = re.match(r"^(\s*)- ", line)
        if match and (indent is None or len(match.group(1)) <= indent):
            if current:
                blocks.append("\n".join(current))
            current, indent = [line], len(match.group(1))
        elif current:
            if line.strip() and len(line) - len(line.lstrip()) <= (indent or 0) and not line.lstrip().startswith("#"):
                blocks.append("\n".join(current))
                current, indent = [], None
            else:
                current.append(line)
    if current:
        blocks.append("\n".join(current))
    return blocks


class PnpmInstallSaysWhereItStopped(unittest.TestCase):
    """An install of pnpm on the self-hosted runner twice ended with exit code 1 and no message (runs 45708835457
    and 46044602286). Each install step writes every line as it comes and says its exit code, in PowerShell, whose
    syntax the lines are."""

    def installs(self):
        found = [
            block
            for block in steps(WORKFLOW.read_text(encoding="utf-8"))
            if re.search(r"^\s*(- )?(run: \|?\s*)?.*pnpm\b.*\binstall\b", block, re.M)
            and "run:" in block
        ]
        self.assertGreaterEqual(len(found), 2, "the install steps of ui-kit and web-kit")
        return found

    def test_every_install_writes_each_line_and_says_its_exit_code(self):
        for block in self.installs():
            with self.subTest(block=block.splitlines()[0]):
                self.assertIn("--reporter=append-only", block)
                self.assertIn("$code = $LASTEXITCODE", block)
                self.assertIn("pnpm install ended with exit code $code", block)
                self.assertIn("exit $code", block)
                self.assertRegex(block, r"shell: pwsh\b")

    def test_the_check_sees_an_install_without_it(self):
        bare = "steps:\n  - run: pnpm install --frozen-lockfile\n  - run: pnpm test\n"
        blocks = [b for b in steps(bare) if "install" in b]
        self.assertEqual(len(blocks), 1)
        self.assertNotIn("--reporter=append-only", blocks[0])


if __name__ == "__main__":
    unittest.main()
