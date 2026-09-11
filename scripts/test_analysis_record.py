#!/usr/bin/env python3
import copy
import importlib.util
import json
import unittest
from pathlib import Path

ROOT = Path(__file__).parent
SPEC = importlib.util.spec_from_file_location("validate_analysis_record", ROOT / "validate_analysis_record.py")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class AnalysisRecordTest(unittest.TestCase):
    def setUp(self):
        self.lines = (ROOT / "fixtures" / "analysis_record_v1.jsonl").read_text(encoding="utf-8").splitlines()

    def test_fixture_is_valid(self):
        self.assertEqual(MODULE.validate_lines(self.lines), [])

    def test_rejects_non_monotonic_ply(self):
        docs = [json.loads(line) for line in self.lines]
        docs[2]["ply"] = 0
        self.assertIn("line 3: ply_order", MODULE.validate_lines([json.dumps(doc) for doc in docs]))

    def test_game_end_must_be_final_and_unique(self):
        docs = [json.loads(line) for line in self.lines]
        docs.append({"type": "game_end", "result": "lose"})
        self.assertIn("line 4: game_end", MODULE.validate_lines([json.dumps(doc) for doc in docs]))

    def test_rejects_post_move_shape_and_hashfull(self):
        docs = [json.loads(line) for line in self.lines]
        docs[1]["sfen"] = "bad"
        docs[1]["hashfull"] = 1001
        errors = MODULE.validate_lines([json.dumps(doc) for doc in docs])
        self.assertIn("line 2: sfen", errors)
        self.assertIn("line 2: hashfull", errors)

    def test_rejects_incomplete_record_without_game_end(self):
        docs = [json.loads(line) for line in self.lines[:-1]]
        self.assertIn("missing game_end", MODULE.validate_lines([json.dumps(doc) for doc in docs]))


if __name__ == "__main__":
    unittest.main()
