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

    def test_v2_record_requires_and_accepts_diagnostic_fields(self):
        docs = [json.loads(line) for line in self.lines]
        docs[0]["schema"] = "sekirei.analysis-record.v2"
        for doc in docs[1:]:
            if doc.get("type") == "search":
                doc.update({
                    "score_kind": "cp",
                    "bound": "unknown",
                    "abort_reason": "unknown",
                    "pv_csa": None,
                })
        self.assertEqual(MODULE.validate_lines([json.dumps(doc) for doc in docs]), [])
        del docs[1]["bound"]
        self.assertIn("line 2: keys", MODULE.validate_lines([json.dumps(doc) for doc in docs]))

    def test_v3_records_completed_iteration_and_decision(self):
        docs = [json.loads(line) for line in self.lines]
        docs[0]["schema"] = "sekirei.analysis-record.v3"
        for doc in docs[1:]:
            if doc.get("type") == "search":
                doc.update({
                    "score_kind": "cp", "bound": "unknown", "completed_bound": "exact",
                    "completed_iteration_valid": True, "abort_reason": "budget",
                    "decision": "move", "pv_csa": None, "root_candidates": None,
                    "budget_ms": 1000, "time_left_before_ms": 5000, "byoyomi_ms": 1000,
                })
        self.assertEqual(MODULE.validate_lines([json.dumps(doc) for doc in docs]), [])
        docs[1]["decision"] = "unrecorded"
        self.assertIn("line 2: decision", MODULE.validate_lines([json.dumps(doc) for doc in docs]))

    def test_manifest_link_is_optional_but_hash_is_strict(self):
        docs = [json.loads(line) for line in self.lines]
        docs[0].update({"run_manifest_path": "results/run-manifest.json", "run_manifest_sha256": "a" * 64})
        self.assertEqual(MODULE.validate_lines([json.dumps(doc) for doc in docs]), [])
        docs[0]["run_manifest_sha256"] = "not-a-sha"
        self.assertIn("line 1: header.run_manifest_sha256", MODULE.validate_lines([json.dumps(doc) for doc in docs]))

    def test_v2_timing_fields_are_accepted_and_checked(self):
        docs = [json.loads(line) for line in self.lines]
        docs[0]["schema"] = "sekirei.analysis-record.v2"
        for doc in docs[1:]:
            if doc.get("type") == "search":
                doc.update({
                    "score_kind": "cp", "bound": "unknown", "abort_reason": "none", "pv_csa": None,
                    "budget_ms": 1000, "time_left_before_ms": 5000, "byoyomi_ms": 1000,
                })
        self.assertEqual(MODULE.validate_lines([json.dumps(doc) for doc in docs]), [])
        docs[1]["budget_ms"] = -1
        self.assertIn("line 2: budget_ms", MODULE.validate_lines([json.dumps(doc) for doc in docs]))

        del docs[1]["budget_ms"]
        self.assertIn("line 2: timing_fields_pair", MODULE.validate_lines([json.dumps(doc) for doc in docs]))

    def test_v2_root_candidates_are_optional_but_strictly_validated(self):
        docs = [json.loads(line) for line in self.lines]
        docs[0]["schema"] = "sekirei.analysis-record.v2"
        docs[1].update({
            "score_kind": "cp", "bound": "exact", "abort_reason": "none", "pv_csa": None,
            "root_candidates": [{
                "move_csa": "+7776FU", "score_cp": 12, "score_kind": "cp", "bound": "exact",
                "depth": 2, "nodes": 100, "elapsed_ms": 1, "aborted": False, "abort_reason": "none",
            }],
        })
        for doc in docs[2:]:
            if doc.get("type") == "search":
                doc.update({"score_kind": "cp", "bound": "unknown", "abort_reason": "none", "pv_csa": None})
        self.assertEqual(MODULE.validate_lines([json.dumps(doc) for doc in docs]), [])
        docs[1]["root_candidates"][0]["bound"] = "invalid"
        self.assertIn("line 2: root_candidate_bound", MODULE.validate_lines([json.dumps(doc) for doc in docs]))

    def test_rejects_non_monotonic_ply(self):
        docs = [json.loads(line) for line in self.lines]
        docs[2]["ply"] = 0
        self.assertIn("line 3: ply_order", MODULE.validate_lines([json.dumps(doc) for doc in docs]))

    def test_v2_rejects_pv_that_disagrees_with_bestmove(self):
        docs = [json.loads(line) for line in self.lines]
        docs[0]["schema"] = "sekirei.analysis-record.v2"
        docs[1]["pv_csa"] = ["-3334FU"]
        docs[1].update({"score_kind": "cp", "bound": "unknown", "abort_reason": "unknown"})
        errors = MODULE.validate_lines([json.dumps(doc) for doc in docs])
        self.assertIn("line 2: pv_first_move", errors)

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
