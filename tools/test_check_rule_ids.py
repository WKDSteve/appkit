"""Tests of tools/check-rule-ids.py: `python -m unittest discover tools`."""

import importlib.util
import os
import pathlib
import subprocess
import tempfile
import unittest

spec = importlib.util.spec_from_file_location(
    "check_rule_ids", pathlib.Path(__file__).with_name("check-rule-ids.py")
)
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)

# Built from parts, so that this file does not hold an id itself.
def rule(prefix, number):
    return f"{prefix}-{number}"


class Findings(unittest.TestCase):
    def test_an_id_in_a_comment_a_name_or_a_doc_is_found(self):
        for prefix in check.PREFIXES:
            text = f"// the rule ({rule(prefix, '27')}) says so"
            self.assertEqual(check.findings(text), [(1, rule(prefix, "27"))], prefix)
        self.assertEqual(
            check.findings("a\nit('balances (" + rule("CSS", "09") + ")')"), [(2, rule("CSS", "09"))]
        )

    def test_words_that_are_no_rule_are_not_found(self):
        for text in ["UTF-16", "SHA-256", "x-12", "AA-01", "LUA-12", "GIT-8", "SRV-123", "see CSS-", "WEB-1x"]:
            self.assertEqual(check.findings(text), [], text)

    def test_the_prefix_is_a_whole_word(self):
        self.assertEqual(check.findings("XSRV-27 and SRV-27x"), [])


class ARepository(unittest.TestCase):
    def run_in(self, files):
        with tempfile.TemporaryDirectory() as root:
            subprocess.run(["git", "init", "-q", root], check=True)
            for name, text in files.items():
                path = pathlib.Path(root, name)
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(text, encoding="utf-8", newline="")
            subprocess.run(["git", "-C", root, "add", "-A"], check=True)
            return check.main(["--root", root])

    def test_a_tracked_file_with_an_id_fails_and_a_clean_one_passes(self):
        self.assertEqual(self.run_in({"a.rs": "// clean\n", "b.md": "text\n"}), 0)
        self.assertEqual(self.run_in({"a.rs": "// clean\n", "b.md": f"see {rule('SRV', '26')}\n"}), 1)

    def test_a_lock_file_and_a_binary_file_are_skipped(self):
        self.assertEqual(self.run_in({"Cargo.lock": f"{rule('SRV', '26')}\n"}), 0)
        with tempfile.TemporaryDirectory() as root:
            subprocess.run(["git", "init", "-q", root], check=True)
            pathlib.Path(root, "x.bin").write_bytes(b"\xff\xfe" + rule("SRV", "26").encode())
            subprocess.run(["git", "-C", root, "add", "-A"], check=True)
            self.assertEqual(check.main(["--root", root]), 0)


if __name__ == "__main__":
    unittest.main()
