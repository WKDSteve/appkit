"""Tests of check-code-ref.py against the golden examples of schemas/code-ref/1, and that schema and reference agree.

    python -m unittest discover tools
"""

import importlib.util
import json
import unittest
from pathlib import Path

HERE = Path(__file__).parent
SPEC = importlib.util.spec_from_file_location("check_code_ref", HERE / "check-code-ref.py")
ref = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ref)

SCHEMA_DIR = HERE.parent / "schemas" / "code-ref" / "1"
EXAMPLES = SCHEMA_DIR / "examples"
JSON_GOLDENS = ("marking", "function", "minimal")


def example(name):
    return json.loads((EXAMPLES / f"{name}.json").read_text(encoding="utf-8"))


def encode(doc):
    return json.dumps(doc).encode("utf-8")


class Goldens(unittest.TestCase):
    def test_each_golden_passes(self):
        self.assertEqual(sorted(p.stem for p in EXAMPLES.glob("*.json")), sorted(JSON_GOLDENS))
        for name in JSON_GOLDENS:
            with self.subTest(name):
                self.assertEqual(ref.problems((EXAMPLES / f"{name}.json").read_bytes()), [])

    def test_the_function_golden_carries_everything_and_the_minimal_one_nothing_more(self):
        self.assertTrue({"sha", "symbol", "docmap", "telemetry", "excerpt", "class"} <= set(example("function")))
        self.assertEqual(set(example("minimal")), {"schema", "repo", "file", "lines"})

    def test_schema_and_reference_agree(self):
        schema = json.loads((SCHEMA_DIR / "code-ref.schema.json").read_text(encoding="utf-8"))
        props = schema["properties"]
        self.assertEqual(props["schema"]["const"], ref.SCHEMA)
        self.assertEqual(tuple(props["class"]["enum"]), ref.CLASSES)
        self.assertEqual(props["file"]["maxLength"], ref.MAX_PATH)
        self.assertEqual(props["excerpt"]["maxLength"], ref.MAX_EXCERPT_BYTES)
        self.assertEqual(props["symbol"]["maxLength"], ref.LIMITS["symbol"])
        self.assertEqual(props["docmap"]["properties"]["node"]["maxLength"], ref.LIMITS["docmap.node"])
        self.assertEqual(props["docmap"]["properties"]["fn"]["maxLength"], ref.LIMITS["docmap.fn"])
        self.assertEqual(props["telemetry"]["properties"]["namespace"]["maxLength"], ref.LIMITS["telemetry.namespace"])
        self.assertEqual(props["telemetry"]["properties"]["key"]["maxLength"], ref.LIMITS["telemetry.key"])
        self.assertEqual(props["repo"]["pattern"], "^" + ref.REPO.pattern + "$")
        self.assertEqual(props["lang"]["pattern"], "^" + ref.LANG.pattern + "$")
        self.assertEqual(props["sha"]["pattern"], "^" + ref.SHA.pattern + "$")
        self.assertEqual(props["lines"]["prefixItems"][0]["maximum"], ref.MAX_LINE)
        self.assertEqual(set(schema["required"]), {"schema", "repo", "file", "lines"})


class Refused(unittest.TestCase):
    def refused(self, doc, needle):
        found = ref.problems(encode(doc))
        self.assertTrue(any(needle in line for line in found), f"{needle!r} not in {found}")

    def test_size_is_measured_before_parsing(self):
        self.assertEqual(ref.problems(b" " * (ref.MAX_BYTES + 1)), [f"larger than {ref.MAX_BYTES} bytes"])

    def test_not_a_document(self):
        for raw in (b"", b"[1]", b"\xff\xfe", b"null", b"[" * 100000):
            with self.subTest(raw[:8]):
                self.assertTrue(ref.problems(raw))

    def test_a_higher_or_foreign_version(self):
        for schema in ("code-ref/2", "code-ref/0", "wkd-intent/1", None, ["code-ref/1"]):
            with self.subTest(schema):
                doc = example("function")
                doc["schema"] = schema
                self.refused(doc, "is not code-ref/1")

    def test_required_fields(self):
        for key in ("repo", "file", "lines"):
            doc = example("minimal")
            del doc[key]
            self.refused(doc, key)

    def test_repo_is_a_name_not_a_path(self):
        for text in ("E:\\repos\\x", "/home/x", "a/b", "..", "", ".x", "x" * 101, 1):
            with self.subTest(text):
                doc = example("minimal")
                doc["repo"] = text
                self.refused(doc, "repo")

    def test_no_way_out_of_the_repository(self):
        escapes = ("/etc/passwd", "\\windows", "C:\\x", "c:/x", "\\\\host\\x", "~/x", "$HOME/x", "${HOME}/x",
                   "%USERPROFILE%\\x", "../x", "a/../../x", "a\\..\\x", "..")
        for text in escapes:
            with self.subTest(text):
                doc = example("minimal")
                doc["file"] = text
                self.refused(doc, "file leaves")
                doc = example("function")
                doc["docmap"]["node"] = text
                self.refused(doc, "docmap.node leaves")

    def test_a_file_is_one_canonical_form(self):
        for text in ("a\\b.lua", "a/b.lua:5", "./a.lua", "a//b.lua", "a/./b.lua", "a/", "x\0y"):
            with self.subTest(text):
                doc = example("minimal")
                doc["file"] = text
                self.refused(doc, "file is not")
        for text in ("a/b.lua", "TESTS/x..y/z.lua", "a..b", "ui-kit/src/a b.css", ".github/workflows/ci.yml"):
            with self.subTest(text):
                doc = example("minimal")
                doc["file"] = text
                self.assertEqual(ref.problems(encode(doc)), [])

    def test_lines(self):
        bad = ([5, 4], [0, 3], [-1, 2], [1], [1, 2, 3], ["1", 2], [1.5, 2], [True, True], "1-2", None, [1, 10_000_001])
        for lines in bad:
            with self.subTest(lines):
                doc = example("minimal")
                doc["lines"] = lines
                self.refused(doc, "lines")
        doc = example("minimal")
        doc["lines"] = [10_000_000, 10_000_000]
        self.assertEqual(ref.problems(encode(doc)), [])

    def test_sha(self):
        for text in ("abc", "0123456789ABCDEF0123456789ABCDEF01234567", "g" * 40, 1):
            with self.subTest(text):
                doc = example("function")
                doc["sha"] = text
                self.refused(doc, "sha")
        for ok in (None, "a" * 40, "b" * 64):
            doc = example("function")
            doc["sha"] = ok
            self.assertEqual(ref.problems(encode(doc)), [], ok)

    def test_only_released_classes(self):
        for cls in ("private", "unclassified", None, 1):
            doc = example("function")
            doc["class"] = cls
            self.refused(doc, "class")
        doc = example("minimal")
        self.assertNotIn("class", doc)
        self.assertEqual(ref.problems(encode(doc)), [])

    def test_excerpt_is_counted_in_bytes(self):
        doc = example("function")
        doc["excerpt"] = "x" * ref.MAX_EXCERPT_BYTES
        self.assertEqual(ref.problems(encode(doc)), [])
        doc["excerpt"] += "x"
        self.refused(doc, "excerpt is longer")
        # 1366 characters, 4098 bytes: short by characters, too long by bytes.
        doc["excerpt"] = "\u20ac" * 1366
        self.refused(doc, "excerpt is longer")
        doc["excerpt"] = "\u20ac" * 1365
        self.assertEqual(ref.problems(encode(doc)), [])
        doc["excerpt"] = 5
        self.refused(doc, "excerpt is not a string")

    def test_excerpt_is_code_not_a_path(self):
        doc = example("function")
        doc["excerpt"] = 'local p = "/etc/passwd" .. "../x" -- ~/y'
        self.assertEqual(ref.problems(encode(doc)), [])

    def test_lengths_and_types_of_the_small_fields(self):
        cases = (("symbol", None, 200), ("docmap", "node", 300), ("docmap", "fn", 200),
                 ("telemetry", "namespace", 100), ("telemetry", "key", 300))
        for key, sub, limit in cases:
            with self.subTest((key, sub)):
                doc = example("function")
                target = doc if sub is None else doc[key]
                field = key if sub is None else sub
                target[field] = "x" * (limit + 1)
                self.refused(doc, f"{key if sub is None else key + '.' + sub}")
                target[field] = "x" * limit
                self.assertEqual(ref.problems(encode(doc)), [])
        doc = example("function")
        doc["docmap"] = {"fn": "node"}
        self.refused(doc, "docmap.node")
        doc = example("function")
        doc["telemetry"] = {"namespace": "x"}
        self.refused(doc, "telemetry.key")
        doc = example("function")
        doc["telemetry"] = "x"
        self.refused(doc, "telemetry is not an object")
        doc = example("function")
        doc["dirty"] = "no"
        self.refused(doc, "dirty")
        doc = example("function")
        doc["lang"] = "Lua!"
        self.refused(doc, "lang")

    def test_control_characters(self):
        for key in ("symbol", "excerpt", "repo"):
            doc = example("function")
            doc[key] = "evil\x1b[31m"
            self.refused(doc, "control character")
        doc = example("function")
        doc["excerpt"] = "line one\r\nline two\ttab"
        self.assertEqual(ref.problems(encode(doc)), [])
        doc["later"] = "a\x00b"
        self.refused(doc, "control character")

    def test_deep_nesting_never_raises(self):
        for depth in (500, 900, 950, 990, 1100, 20000):
            with self.subTest(depth):
                raw = b'{"schema":"code-ref/1","later":' + b"[" * depth + b"]" * depth + b"}"
                self.assertIsInstance(ref.problems(raw), list)

    def test_unknown_fields_are_ignored(self):
        doc = example("function")
        doc["later"] = {"x": 1}
        doc["docmap"]["later"] = 1
        self.assertEqual(ref.problems(encode(doc)), [])


class Blocks(unittest.TestCase):
    BODY = (EXAMPLES / "task-body.md").read_text(encoding="utf-8")

    def test_the_golden_task_body_yields_its_two_blocks_and_both_pass(self):
        found = ref.task_problems(self.BODY)
        self.assertEqual([i for i, _ in found], [0, 1])
        self.assertEqual([p for _, p in found], [[], []])

    def test_only_the_language_code_ref_counts(self):
        text = '```json\n{"schema": "code-ref/1"}\n```\n```code-ref-x\n{}\n```\n'
        self.assertEqual(ref.blocks(text), [])

    def test_a_reference_inside_a_longer_fence_is_shown_not_read(self):
        text = "````md\n```code-ref\n{}\n```\n````\n"
        self.assertEqual(ref.blocks(text), [])
        text = "```\n```code-ref\n{}\n```\n"
        self.assertEqual(ref.blocks(text), [])

    def test_frontmatter_is_not_body_and_an_unclosed_block_is_dropped(self):
        text = "---\nnote: |\n  ```code-ref\n---\nbody\n```code-ref\n{}\n"
        self.assertEqual(ref.blocks(text), [])

    def test_a_bad_block_is_reported_with_its_number(self):
        text = "```code-ref\n" + json.dumps(example("minimal")) + "\n```\n```code-ref\n{\"schema\": \"code-ref/2\"}\n```\n"
        found = ref.task_problems(text)
        self.assertEqual(found[0][1], [])
        self.assertTrue(found[1][1])

    def test_crlf_and_indented_fences(self):
        text = "  ```code-ref\r\n{\"a\": 1}\r\n  ```\r\n"
        self.assertEqual(ref.blocks(text), ['{"a": 1}'])

    def test_each_block_has_its_own_size_limit(self):
        text = "```code-ref\n" + " " * (ref.MAX_BYTES + 1) + "\n```\n"
        self.assertEqual(ref.task_problems(text), [(0, [f"larger than {ref.MAX_BYTES} bytes"])])


if __name__ == "__main__":
    unittest.main()
