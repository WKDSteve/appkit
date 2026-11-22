"""Tests of pick_target.pick: which commit the rolling build `latest` is made from (rule GIT-08).

    python -m unittest discover -s release-kit -p "test_*.py" -v
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import pick_target  # noqa: E402


def c(n: int) -> str:
    return f"{n:040x}"


# The branch, newest first: c(5) is the tip.
BRANCH = [c(5), c(4), c(3), c(2), c(1)]


def descendant(old: str, new: str) -> bool:
    """On BRANCH a commit is a descendant of every older one (a line of history)."""
    return BRANCH.index(new) <= BRANCH.index(old)


class Pick(unittest.TestCase):
    def test_red_tip_builds_the_green_commit_before_it(self):
        target, _ = pick_target.pick(BRANCH, {c(4), c(2)}, c(2), descendant)
        self.assertEqual(target, c(4))

    def test_running_tip_counts_as_not_green(self):
        # c(5) has no finished run yet: the newest green one is built, not nothing.
        target, _ = pick_target.pick(BRANCH, {c(3)}, None, descendant)
        self.assertEqual(target, c(3))

    def test_green_tip_is_built(self):
        target, _ = pick_target.pick(BRANCH, {c(5), c(4)}, c(3), descendant)
        self.assertEqual(target, c(5))

    def test_already_built_builds_nothing(self):
        target, why = pick_target.pick(BRANCH, {c(5)}, c(5), descendant)
        self.assertIsNone(target)
        self.assertIn("built already", why)

    def test_target_older_than_latest_builds_nothing(self):
        # latest is at c(4) (built from an earlier decision); the only green commit now is older.
        target, why = pick_target.pick(BRANCH, {c(2)}, c(4), descendant)
        self.assertIsNone(target)
        self.assertIn("only moves forward", why)

    def test_no_green_commit_builds_nothing(self):
        target, _ = pick_target.pick(BRANCH, set(), c(2), descendant)
        self.assertIsNone(target)

    def test_first_build_needs_no_ancestry(self):
        def never(_old: str, _new: str) -> bool:
            raise AssertionError("no built commit: nothing to compare with")

        target, _ = pick_target.pick(BRANCH, {c(3)}, None, never)
        self.assertEqual(target, c(3))

    def test_requested_green_commit_may_be_older_than_the_newest_green(self):
        target, _ = pick_target.pick(BRANCH, {c(4), c(3)}, c(2), descendant, requested=c(3))
        self.assertEqual(target, c(3))

    def test_requested_commit_can_rebuild_the_built_one(self):
        target, _ = pick_target.pick(BRANCH, {c(3)}, c(3), descendant, requested=c(3))
        self.assertEqual(target, c(3))

    def test_requested_red_commit_is_refused(self):
        target, why = pick_target.pick(BRANCH, {c(3)}, c(2), descendant, requested=c(4))
        self.assertIsNone(target)
        self.assertIn("no green CI run", why)

    def test_requested_commit_still_only_moves_forward(self):
        target, _ = pick_target.pick(BRANCH, {c(2), c(4)}, c(4), descendant, requested=c(2))
        self.assertIsNone(target)

    def test_requested_commit_off_the_branch_is_refused(self):
        target, _ = pick_target.pick(BRANCH, {c(9)}, None, descendant, requested=c(9))
        self.assertIsNone(target)


if __name__ == "__main__":
    unittest.main()
