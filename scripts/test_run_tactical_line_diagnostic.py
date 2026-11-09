#!/usr/bin/env python3
"""Focused tests for the tactical-line diagnostic contract."""

from __future__ import annotations

import importlib.util
import io
import json
import sys
import tempfile
import unittest
import subprocess
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
SPEC = importlib.util.spec_from_file_location(
    "tactical", ROOT / "scripts" / "run_tactical_line_diagnostic.py"
)
assert SPEC and SPEC.loader
TACTICAL = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(TACTICAL)


def result_fields(score: int = 42, *, bestmove: str = "8g8f") -> dict[str, str]:
    return {
        "bestmove": bestmove,
        "depth": "8",
        "score_cp": str(score),
        "nodes": "1000",
        "elapsed_ms": "2",
        "bound": "unknown",
        "completed_bound": "exact",
        "completed_iteration_valid": "true",
        "aborted": "true",
        "abort_reason": "budget",
        "pv_usi": f"{bestmove},5c5d",
        "pv_legal": "true",
        "history_matches_expected": "true",
    }


class TacticalLineDiagnosticTests(unittest.TestCase):
    def test_deadline_failure_saves_partial_row_without_ranking_it(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            engine, history = root / "engine", root / "history"
            engine.touch()
            history.touch()
            sfen = "9/9/9/9/9/9/9/9/9 b - 1"
            cases = root / "cases.json"
            cases.write_text(json.dumps({"schema": TACTICAL.SCHEMA, "cases": [{
                "id": "case", "sfen": sfen,
                "stages": [{"id": "first", "candidates": ["7g7f"]}]
            }]}), encoding="utf-8")
            output = root / "report.json"
            fields = result_fields()
            search_line = "\t".join(f"{key}={value}" for key, value in fields.items())
            calls = [
                subprocess.CompletedProcess([], 0, f"sfen={sfen}\thash=abc\n", ""),
                subprocess.CompletedProcess([], 0, search_line + "\n", ""),
                subprocess.TimeoutExpired("engine", 1),
            ]
            argv = ["diagnostic", "--cases", str(cases), "--engine", str(engine),
                    "--history-sfen", str(history), "--nodes", "1000",
                    "--max-invocations", "3", "--max-total-seconds", "1",
                    "--output", str(output)]
            with patch.object(sys, "argv", argv), patch.object(
                TACTICAL, "run_process", side_effect=calls
            ), redirect_stdout(io.StringIO()):
                self.assertEqual(TACTICAL.main(), 1)
            report = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(report["status"], "interrupted")
            self.assertEqual(report["completed_rows"], 0)
            self.assertEqual(report["rows"], [])
            self.assertEqual(report["partial_row"]["unrestricted"]["bestmove"], "8g8f")
            self.assertEqual(report["partial_row"]["fixed_root_candidates"], [])
            self.assertEqual(report["failure"]["reason"], "TimeoutExpired")

    def test_pruning_counts_and_lmr_ablation_are_explicit_and_parsed(self) -> None:
        fields = result_fields()
        keys = (
            "v2_lmp_skips", "v2_quiet_history_skips", "v2_quiet_futility_skips",
            "v2_quiet_see_skips", "v2_tactical_futility_skips",
            "v2_tactical_see_skips", "v2_reduced_children",
        )
        fields.update({key: str(index) for index, key in enumerate(keys)})
        output = "\t".join(f"{key}={value}" for key, value in fields.items())
        with patch.object(TACTICAL, "run_process", return_value=subprocess.CompletedProcess([], 0, output, "")) as run:
            observed = TACTICAL.search(Path("engine"), "sfen", [], "sfen", 1000, None, "absolute", None,
                                       disable_lmr=True, pruning_counts=True)
        self.assertEqual(run.call_args.args[0][-2:], ["--disable-lmr", "--pruning-counts"])
        self.assertEqual(observed["v2_move_decisions"]["v2_reduced_children"], 6)

    def test_interrupted_bestmove_from_deeper_iteration_is_not_ranked(self) -> None:
        fields = result_fields(bestmove="8g8f")
        fields["pv_usi"] = "7g7f,3c3d"
        fields["pv_root"] = "7g7f"
        fields["completed_bestmove"] = "7g7f"
        fields["bestmove_matches_pv_root"] = "false"
        result = TACTICAL.encode_result(fields, None)
        self.assertFalse(result["bestmove_matches_pv_root"])
        self.assertEqual(result["pv_root"], "7g7f")
        self.assertEqual(result["completed_bestmove"], "7g7f")
        TACTICAL.rank_candidates([result])
        self.assertIsNone(result["diagnostic_rank"])
        fields["completed_bestmove"] = "8g8f"
        with self.assertRaisesRegex(ValueError, "inconsistent completed bestmove"):
            TACTICAL.encode_result(fields, None)
        fields["completed_bestmove"] = "7g7f"
        fields["bestmove_matches_pv_root"] = "true"
        with self.assertRaisesRegex(ValueError, "inconsistent bestmove/PV"):
            TACTICAL.encode_result(fields, None)

    def test_invocation_cap_rejects_batch_before_any_subprocess(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            engine = root / "engine"
            history = root / "history"
            engine.touch()
            history.touch()
            cases = root / "cases.json"
            cases.write_text(json.dumps({
                "schema": TACTICAL.SCHEMA,
                "cases": [{
                    "id": "case",
                    "sfen": "9/9/9/9/9/9/9/9/9 b - 1",
                    "stages": [
                        {"id": "one", "candidates": ["7g7f"]},
                        {"id": "two", "candidates": ["2g2f"]},
                    ],
                }],
            }), encoding="utf-8")
            argv = ["diagnostic", "--cases", str(cases), "--engine", str(engine),
                    "--history-sfen", str(history), "--nodes", "50000",
                    "--max-invocations", "4", "--output", str(root / "report.json")]
            stderr = io.StringIO()
            with patch.object(sys, "argv", argv), patch.object(
                TACTICAL, "run_process", side_effect=AssertionError("subprocess started")
            ), redirect_stderr(stderr), self.assertRaises(SystemExit) as raised:
                TACTICAL.main()
            self.assertEqual(raised.exception.code, 2)
            self.assertIn("requires 6 subprocesses", stderr.getvalue())

    def test_shipped_case_replays_reference_and_alternative_positions(self) -> None:
        document = TACTICAL.load_cases(
            ROOT / "scripts" / "fixtures" / "tactical_line_cases_v1.json"
        )
        stages = document["cases"][0]["stages"]
        self.assertEqual(stages[1]["candidates"], ["P*8g", "8e8f"])
        self.assertEqual(stages[1]["reference_moves"], ["P*8g"])
        self.assertEqual(stages[2]["moves"], ["8g8f", "P*8g"])
        self.assertEqual(stages[3]["moves"], ["8g8f", "8e8f"])

    def test_reference_move_must_be_an_explicit_candidate(self) -> None:
        invalid = {
            "schema": TACTICAL.SCHEMA,
            "cases": [
                {
                    "id": "case",
                    "sfen": "9/9/9/9/9/9/9/9/9 b - 1",
                    "stages": [
                        {
                            "id": "stage",
                            "moves": [],
                            "candidates": ["7g7f"],
                            "reference_moves": ["2g2f"],
                        }
                    ],
                }
            ],
        }
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "cases.json"
            path.write_text(json.dumps(invalid), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "explicit candidates"):
                TACTICAL.load_cases(path)

    def test_rank_is_withheld_when_any_candidate_is_incomplete(self) -> None:
        complete = TACTICAL.encode_result(result_fields(100, bestmove="P*8g"), "P*8g")
        incomplete_fields = result_fields(200, bestmove="8e8f")
        incomplete_fields["completed_iteration_valid"] = "false"
        incomplete = TACTICAL.encode_result(incomplete_fields, "8e8f")
        results = [complete, incomplete]
        TACTICAL.rank_candidates(results)
        self.assertEqual([result["diagnostic_rank"] for result in results], [None, None])

    def test_complete_candidates_are_ranked_without_asserting_reference(self) -> None:
        lower = TACTICAL.encode_result(result_fields(-20, bestmove="P*8g"), "P*8g")
        higher = TACTICAL.encode_result(result_fields(30, bestmove="8e8f"), "8e8f")
        results = [lower, higher]
        TACTICAL.rank_candidates(results)
        self.assertEqual(results[0]["diagnostic_rank"], 2)
        self.assertEqual(results[1]["diagnostic_rank"], 1)
        self.assertEqual(results[0]["score_kind"], "cp")

    def test_fixed_root_result_must_retain_requested_move(self) -> None:
        with self.assertRaisesRegex(ValueError, "fixed-root search returned"):
            TACTICAL.encode_result(result_fields(bestmove="8e8f"), "P*8g")

    def test_atomic_output_replaces_with_valid_json(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "nested" / "report.json"
            TACTICAL.atomic_json(output, {"schema": TACTICAL.REPORT_SCHEMA, "rows": []})
            self.assertEqual(json.loads(output.read_text(encoding="utf-8"))["rows"], [])

    def test_side_to_move_tracks_replayed_ply(self) -> None:
        sfen = "9/9/9/9/9/9/9/9/9 b - 1"
        self.assertEqual(TACTICAL.side_to_move(sfen, []), "black")
        self.assertEqual(TACTICAL.side_to_move(sfen, ["7g7f"]), "white")


if __name__ == "__main__":
    unittest.main()
