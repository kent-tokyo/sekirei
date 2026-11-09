"""Fail-closed tests for the bounded V2 move trace runner."""

from __future__ import annotations

import importlib.util
import io
import json
import sys
import tempfile
import unittest
from contextlib import redirect_stderr
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
SPEC = importlib.util.spec_from_file_location(
    "v2_trace", ROOT / "scripts" / "run_v2_move_trace_diagnostic.py"
)
assert SPEC and SPEC.loader
TRACE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(TRACE)


class V2MoveTraceTests(unittest.TestCase):
    def test_invocation_limit_rejects_before_search(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            engine = Path(directory) / "engine"
            engine.touch()
            argv = ["trace", "--engine", str(engine), "--sfen",
                    "4kg3/9/9/9/9/9/9/1R7/4K4 b P 1", "--root-move", "P*4c",
                    "--reply", "4a3a", "--reply", "4a3b", "--nodes", "20000",
                    "--max-invocations", "2", "--output", str(Path(directory) / "out.json")]
            stderr = io.StringIO()
            with patch.object(sys, "argv", argv), patch.object(
                TRACE, "run_process", side_effect=AssertionError("search started")
            ), redirect_stderr(stderr), self.assertRaises(SystemExit) as raised:
                TRACE.main()
            self.assertEqual(raised.exception.code, 2)
            self.assertIn("requires 3 invocations", stderr.getvalue())

    def test_hard_budget_rejects_before_search(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            engine = Path(directory) / "engine"
            engine.touch()
            argv = ["trace", "--engine", str(engine), "--sfen",
                    "4kg3/9/9/9/9/9/9/1R7/4K4 b P 1", "--root-move", "P*4c",
                    "--reply", "4a3a", "--nodes", "50001",
                    "--max-invocations", "2", "--output", str(Path(directory) / "out.json")]
            stderr = io.StringIO()
            with patch.object(sys, "argv", argv), patch.object(
                TRACE, "run_process", side_effect=AssertionError("search started")
            ), redirect_stderr(stderr), self.assertRaises(SystemExit) as raised:
                TRACE.main()
            self.assertEqual(raised.exception.code, 2)
            self.assertIn("50000 nodes", stderr.getvalue())

    def test_interruption_retains_completed_prefix(self) -> None:
        baseline = {key: value for key, value in zip(
            TRACE.IDENTITY_KEYS, ("P*4c", 1, 4, 100, "exact", "P*4c,4a5b"))}
        traced = {**baseline, "target_hash": "a" * 16,
                  "trace": {key: 0 for key in TRACE.TRACE_KEYS}}
        def search(reply):
            if reply is None:
                return baseline
            if reply == "4a3a":
                return traced
            raise TimeoutError("batch deadline")
        observed_baseline, rows, failure = TRACE.run_trace_batch(
            search, ["4a3a", "4a3b"])
        self.assertEqual(observed_baseline, baseline)
        self.assertEqual([row["reply"] for row in rows], ["4a3a"])
        self.assertTrue(rows[0]["identity_matches_baseline"])
        self.assertEqual(failure["reply"], "4a3b")
        self.assertEqual(failure["reason"], "TimeoutError")

    def test_trace_identity_change_is_recorded(self) -> None:
        baseline = {key: value for key, value in zip(
            TRACE.IDENTITY_KEYS, ("P*4c", 1, 4, 100, "exact", "P*4c,4a5b"))}
        changed = {**baseline, "nodes": 101, "target_hash": "a" * 16,
                   "trace": {key: 0 for key in TRACE.TRACE_KEYS}}
        observed_baseline, rows, failure = TRACE.run_trace_batch(
            lambda reply: baseline if reply is None else changed, ["4a3a", "4a3b"])
        self.assertEqual(observed_baseline, baseline)
        self.assertEqual(len(rows), 1)
        self.assertFalse(rows[0]["identity_matches_baseline"])
        self.assertEqual(failure["reply"], "4a3a")

    def test_main_writes_partial_report_after_timeout(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            engine = Path(directory) / "engine"
            engine.touch()
            output = Path(directory) / "out.json"
            argv = ["trace", "--engine", str(engine), "--sfen",
                    "4kg3/9/9/9/9/9/9/1R7/4K4 b P 1", "--root-move", "P*4c",
                    "--reply", "4a3a", "--nodes", "100", "--max-invocations", "2",
                    "--output", str(output)]
            fields = {"completed_iteration_valid": "true", "completed_bound": "exact",
                      "pv_legal": "true", "pv_replay_preserves_input": "true",
                      "search_board_restored": "true",
                      "bestmove": "P*4c", "score_cp": "1", "depth": "4", "nodes": "100",
                      "pv_usi": "P*4c,4a5b"}
            with patch.object(sys, "argv", argv), patch.object(
                TRACE, "run_process", side_effect=[SimpleNamespace(stdout="ok"),
                                                    TimeoutError("deadline")]
            ), patch.object(TRACE, "parse_fields", return_value=fields):
                self.assertEqual(TRACE.main(), 1)
            report = json.loads(output.read_text())
            self.assertEqual(report["status"], "interrupted")
            self.assertEqual(report["completed_invocations"], 1)
            self.assertEqual(report["failure"]["reply"], "4a3a")

    def test_incomplete_or_mismatched_trace_is_rejected(self) -> None:
        fields = {
            "completed_iteration_valid": "true", "completed_bound": "exact",
            "pv_legal": "true", "pv_replay_preserves_input": "true",
            "search_board_restored": "true",
            "bestmove": "P*4c", "score_cp": "1", "depth": "4", "nodes": "100",
            "pv_usi": "P*4c,4a5b", "trace_target_move": "4a5b",
            "trace_target_hash": "0123456789abcdef",
        }
        fields.update({key: "0" for key in TRACE.TRACE_KEYS})
        fields["search_board_restored"] = "false"
        with self.assertRaisesRegex(ValueError, "mutated input"):
            TRACE.parse_result(fields, "4a5b")
        fields["search_board_restored"] = "true"
        with self.assertRaisesRegex(ValueError, "target differs"):
            TRACE.parse_result(fields, "4a3b")
        fields["completed_bound"] = "unknown"
        with self.assertRaisesRegex(ValueError, "no completed exact"):
            TRACE.parse_result(fields, "4a5b")


if __name__ == "__main__":
    unittest.main()
