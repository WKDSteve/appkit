"""Tests of check-wkd-intent.py against the golden examples of schemas/wkd-intent/1, and that schema and reference agree.

    python -m unittest discover tools
"""

import importlib.util
import json
import unittest
from pathlib import Path

HERE = Path(__file__).parent
SPEC = importlib.util.spec_from_file_location("check_wkd_intent", HERE / "check-wkd-intent.py")
intent = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(intent)

SCHEMA_DIR = HERE.parent / "schemas" / "wkd-intent" / "1"
EXAMPLES = SCHEMA_DIR / "examples"


def example(name):
    return json.loads((EXAMPLES / f"{name}.json").read_text(encoding="utf-8"))


def encode(doc):
    return json.dumps(doc).encode("utf-8")


class Goldens(unittest.TestCase):
    def test_there_is_one_per_action_and_each_passes(self):
        names = sorted(path.stem for path in EXAMPLES.glob("*.json"))
        self.assertEqual(names, sorted(intent.ACTIONS))
        for name in names:
            with self.subTest(name):
                raw = (EXAMPLES / f"{name}.json").read_bytes()
                self.assertEqual(intent.problems(raw), [])
                self.assertEqual(json.loads(raw)["action"], name)

    def test_schema_names_the_same_actions_modes_and_classes(self):
        schema = json.loads((SCHEMA_DIR / "wkd-intent.schema.json").read_text(encoding="utf-8"))
        self.assertEqual(tuple(schema["properties"]["action"]["enum"]), intent.ACTIONS)
        self.assertEqual(tuple(schema["properties"]["mode"]["enum"]), intent.MODES)
        self.assertEqual(schema["properties"]["schema"]["const"], intent.SCHEMA)
        finding = schema["$defs"]["finding"]["properties"]
        self.assertEqual(tuple(finding["class"]["enum"]), intent.CLASSES)
        for key, limit in intent.LIMITS.items():
            self.assertEqual(finding[key]["maxLength"], limit, key)


class Refused(unittest.TestCase):
    def refused(self, doc, needle, accepted=intent.ACTIONS):
        found = intent.problems(encode(doc), accepted)
        self.assertTrue(any(needle in line for line in found), f"{needle!r} not in {found}")

    def test_size_is_measured_before_parsing(self):
        self.assertEqual(intent.problems(b" " * (intent.MAX_BYTES + 1)), [f"larger than {intent.MAX_BYTES} bytes"])

    def test_not_a_document(self):
        for raw in (b"", b"[1]", b"\xff\xfe", b"null", b"[" * 100000):
            with self.subTest(raw[:8]):
                self.assertTrue(intent.problems(raw))

    def test_a_higher_version(self):
        doc = example("tasks.new")
        doc["schema"] = "wkd-intent/2"
        self.refused(doc, "is not wkd-intent/1")

    def test_an_action_the_target_did_not_offer(self):
        self.refused(example("rules.new"), "does not take rules.new", ("tasks.new", "tasks.show"))
        doc = example("tasks.new")
        doc["action"] = "tasks.delete"
        self.refused(doc, "unknown action")
        doc["action"] = ["tasks.new"]
        self.refused(doc, "unknown action")

    def test_shape_of_the_action(self):
        doc = example("tasks.show")
        doc["mode"] = "agent"
        self.refused(doc, "only knows mode form")
        doc = example("tasks.show")
        doc["finding"] = example("tasks.new")["finding"]
        self.refused(doc, "carries no finding")
        doc = example("tasks.show")
        doc["target"] = None
        self.refused(doc, "target.id is missing")
        doc = example("tasks.new")
        doc["finding"] = None
        self.refused(doc, "needs a finding")
        doc = example("tasks.new")
        doc["target"] = {"id": "a/b"}
        self.refused(doc, "has no target")
        doc = example("tasks.new")
        del doc["target"]
        self.refused(doc, "target is missing")

    def test_only_released_classes(self):
        for cls in ("private", "unclassified", None, 1):
            doc = example("tasks.new")
            doc["finding"]["class"] = cls
            self.refused(doc, "finding.class")

    def test_no_way_out_of_the_repository(self):
        escapes = (
            "/etc/passwd",
            "\\windows",
            "C:\\x",
            "c:/x",
            "\\\\host\\x",
            "~/x",
            "$HOME/x",
            "${HOME}/x",
            "%USERPROFILE%\\x",
            "../x",
            "a/../../x",
            "a\\..\\x",
            "..",
        )
        for text in escapes:
            with self.subTest(text):
                doc = example("tasks.new")
                doc["finding"]["where"]["file"] = text
                self.refused(doc, "where.file leaves")
                doc = example("tasks.new")
                doc["finding"]["repro"] = f"cat {text}"
                self.refused(doc, "repro leaves")
                doc = example("tasks.show")
                doc["target"]["id"] = text
                self.refused(doc, "target.id leaves")
        doc = example("tasks.new")
        doc["finding"]["repro"] = 'tool --out="/etc/x"'
        self.refused(doc, "repro leaves")
        doc["finding"]["repro"] = "tool --out=/etc/x"
        self.refused(doc, "repro leaves")

    def test_relative_paths_pass(self):
        for text in ("a/b.lua", "TESTS/x..y/z.lua", "a..b", "scripts/test.sh --file x"):
            with self.subTest(text):
                doc = example("tasks.new")
                doc["finding"]["repro"] = text
                self.assertEqual(intent.problems(encode(doc)), [])

    def test_lengths(self):
        for key, limit in intent.LIMITS.items():
            doc = example("tasks.new")
            doc["finding"][key] = "x" * (limit + 1)
            self.refused(doc, f"finding.{key} is longer")
            doc["finding"][key] = "x" * limit
            self.assertEqual(intent.problems(encode(doc)), [], key)
        doc = example("tasks.show")
        doc["target"]["id"] = "x" * 301
        self.refused(doc, "target.id is longer")

    def test_control_characters_and_a_foreign_url(self):
        doc = example("tasks.new")
        doc["finding"]["title"] = "evil\x1b[31m"
        self.refused(doc, "control character")
        doc = example("tasks.new")
        doc["finding"]["excerpt"] = "line one\nline two\ttab"
        self.assertEqual(intent.problems(encode(doc)), [])
        doc["finding"]["run"]["url"] = "https://evil.example/x"
        self.refused(doc, "finding.run.url")

    def test_deep_nesting_never_raises(self):
        for depth in (500, 900, 950, 990, 1100, 20000):
            with self.subTest(depth):
                raw = b'{"schema":"wkd-intent/1","later":' + b"[" * depth + b"]" * depth + b"}"
                self.assertIsInstance(intent.problems(raw), list)

    def test_unknown_fields_are_ignored(self):
        doc = example("tasks.new")
        doc["later"] = {"x": 1}
        doc["finding"]["later"] = 1
        self.assertEqual(intent.problems(encode(doc)), [])


if __name__ == "__main__":
    unittest.main()
