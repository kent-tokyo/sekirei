#!/usr/bin/env python3

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "merge_teacher_caches", ROOT / "merge_teacher_caches.py"
)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


def row(sfen, score, identity="nnue:abc", depth=2):
    return {
        "sfen": sfen,
        "label_depth": depth,
        "teacher_identity": identity,
        "score_cp": score,
    }


class MergeTeacherCachesTest(unittest.TestCase):
    def write(self, path, rows):
        path.write_text("".join(json.dumps(item) + "\n" for item in rows), encoding="utf-8")

    def test_merge_sorts_and_deduplicates_equal_scores(self):
        with tempfile.TemporaryDirectory() as tmp:
            left = Path(tmp) / "left.jsonl"
            right = Path(tmp) / "right.jsonl"
            self.write(left, [row("b", 20), row("a", 10)])
            self.write(right, [row("b", 20), row("c", 30)])
            rows, duplicates = MODULE.merge([left, right])
            self.assertEqual([item["sfen"] for item in rows], ["a", "b", "c"])
            self.assertEqual(duplicates, 1)

    def test_conflicting_score_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            left = Path(tmp) / "left.jsonl"
            right = Path(tmp) / "right.jsonl"
            self.write(left, [row("same", 10)])
            self.write(right, [row("same", 11)])
            with self.assertRaisesRegex(ValueError, "conflicting duplicate"):
                MODULE.merge([left, right])

    def test_mixed_contract_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            left = Path(tmp) / "left.jsonl"
            right = Path(tmp) / "right.jsonl"
            self.write(left, [row("a", 10)])
            self.write(right, [row("b", 20, depth=4)])
            with self.assertRaisesRegex(ValueError, "cache contract"):
                MODULE.merge([left, right])


if __name__ == "__main__":
    unittest.main()
