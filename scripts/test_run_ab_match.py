#!/usr/bin/env python3
"""Unit tests for the SPRT helpers in run_ab_match.py (stdlib unittest only).

Run: python3 scripts/test_run_ab_match.py
"""

import hashlib
import io
import json
import math
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))

from run_ab_match import (  # noqa: E402
    classify_terminal_status,
    main,
    sekirei_options,
    sprt_bounds,
    sprt_llr,
    yaneuraou_options,
)


class _FakeMatchProcess:
    """Minimal stdlib-only `Popen` replacement for an early-stop regression."""

    def __init__(self, lines: list[str], exit_code: int = 0):
        self._lines = iter(lines)
        self._exit_code = exit_code
        self.stdout = self
        self.terminated = False
        self.lines_read = 0

    def __iter__(self):
        return self

    def __next__(self):
        line = next(self._lines)
        self.lines_read += 1
        return line

    def terminate(self):
        self.terminated = True

    def wait(self):
        return 143 if self.terminated else self._exit_code


class SprtTest(unittest.TestCase):
    def test_terminal_status_keeps_error_precedence(self):
        status = classify_terminal_status(20, 20, 7, "OSError: broken pipe", "H1 accepted", True)
        self.assertEqual(status.terminal_state, "partial")
        self.assertEqual(status.gate_status, "error")
        self.assertEqual(status.exit_code, 7)

    def test_parallel_options_are_explicit_for_both_engines(self):
        sekirei = sekirei_options(
            "--engine-option1", "weights.nnue", 24, 4, "LazySMP", 0, 128
        )
        self.assertIn("Threads=4", sekirei)
        self.assertIn("SearchMode=LazySMP", sekirei)
        self.assertIn("SpecTopN=0", sekirei)
        self.assertIn("Hash=128", sekirei)

        yaneuraou = yaneuraou_options("weights.nnue", 24, 20_000, 3, 256)
        self.assertIn("Threads=3", yaneuraou)
        self.assertIn("USI_Hash=256", yaneuraou)
        self.assertIn("NodesLimit=20000", yaneuraou)

    def test_material_only_options_omit_evalfile_and_unadvertised_book_option(self):
        options = sekirei_options(
            "--engine-option1", None, 24, 1, "Speculative", 0, 64
        )
        self.assertNotIn("EvalFile=none", options)
        self.assertFalse(any("EvalFile=" in option for option in options))
        self.assertFalse(any("FV_SCALE=" in option for option in options))
        self.assertNotIn("UseBook=false", options)

    def test_bounds_match_wald_for_five_percent_errors(self):
        lower, upper = sprt_bounds()
        self.assertAlmostEqual(lower, math.log(0.05 / 0.95))
        self.assertAlmostEqual(upper, math.log(0.95 / 0.05))

    def test_llr_sign_follows_the_score(self):
        self.assertEqual(sprt_llr(0, 0, 0, 0, 10), 0.0)
        self.assertGreater(sprt_llr(60, 40, 20, 0, 10), 0)
        self.assertLess(sprt_llr(40, 60, 20, 0, 10), 0)

    def test_strong_results_cross_the_bounds(self):
        lower, upper = sprt_bounds()
        # About +70 Elo over 600 games clearly accepts H1 for [0, 10].
        self.assertGreater(sprt_llr(300, 180, 120, 0, 10), upper)
        # About -70 Elo accepts H0.
        self.assertLess(sprt_llr(180, 300, 120, 0, 10), lower)
        # An even result stays between the bounds after 200 games.
        self.assertTrue(lower < sprt_llr(80, 80, 40, 0, 10) < upper)

    def test_llr_scales_with_games_at_a_fixed_score(self):
        small = sprt_llr(60, 40, 20, 0, 10)
        large = sprt_llr(600, 400, 200, 0, 10)
        self.assertAlmostEqual(large, 10 * small)

    def test_main_terminates_match_when_sprt_accepts_h1(self):
        # The process emits a result stream which would accept H1 well before
        # its configured 600-game upper bound.  No engine binary is needed.
        lines = (
            ["→ Engine1 Win\n"] * 300
            + ["→ Engine2 Win\n"] * 180
            + ["→ Draw\n"] * 120
        )
        process = _FakeMatchProcess(lines)
        with tempfile.TemporaryDirectory() as directory:
            result_path = Path(directory) / "gate_result.json"
            argv = [
                "run_ab_match.py",
                "selfplay",
                "--engine-a", "candidate",
                "--engine-b", "baseline",
                "--evalfile", "weights.nnue",
                "--games", "600",
                "--sprt", "0,10",
                "--out-dir", directory,
                "--result-json", str(result_path),
            ]
            argv.extend(["--threads-a", "4", "--threads-b", "2"])
            argv.extend(["--option-a", "T_V2_STAGE_GEN=1"])
            argv.extend(["--option-b", "T_V2_STAGE_GEN=0"])
            with patch.object(sys, "argv", argv), patch(
                "run_ab_match.subprocess.Popen", return_value=process
            ) as popen, patch(
                "run_ab_match.runner_identity", return_value={"commit": "test", "dirty": False}
            ), patch("sys.stdout", new_callable=io.StringIO) as stdout:
                self.assertEqual(main(), 0)

            result = json.loads(result_path.read_text())

        self.assertTrue(process.terminated)
        self.assertLess(process.lines_read, len(lines))
        self.assertIn("H1 accepted", stdout.getvalue())
        self.assertEqual(popen.call_args.kwargs["env"]["RAYON_NUM_THREADS"], "4")
        command = popen.call_args.args[0]
        self.assertIn("Threads=4", command)
        self.assertIn("Threads=2", command)
        self.assertIn("T_V2_STAGE_GEN=1", command)
        self.assertIn("T_V2_STAGE_GEN=0", command)
        self.assertEqual(result["schema_version"], "sekirei.ab_gate_result.v1")
        self.assertEqual(result["terminal_state"], "sprt_stopped")
        self.assertEqual(result["status"], "pass")
        self.assertEqual(result["result"]["sprt"]["verdict"], "accept_h1")
        self.assertTrue(result["evidence"]["complete"])

    def test_main_defaults_to_material_only(self):
        process = _FakeMatchProcess(["→ Draw\n"])
        with tempfile.TemporaryDirectory() as directory:
            argv = [
                "run_ab_match.py",
                "selfplay",
                "--engine-a",
                "candidate",
                "--engine-b",
                "baseline",
                "--games",
                "1",
                "--out-dir",
                directory,
            ]
            with patch.object(sys, "argv", argv), patch(
                "run_ab_match.subprocess.Popen", return_value=process
            ) as popen, patch("sys.stdout", new_callable=io.StringIO):
                self.assertEqual(main(), 0)

        command = popen.call_args.args[0]
        self.assertFalse(any(option.startswith("EvalFile=") for option in command))
        self.assertFalse(any(option.startswith("FV_SCALE=") for option in command))

    def test_result_json_records_normal_completion(self):
        process = _FakeMatchProcess(
            ["→ Engine1 Win\n", "→ Engine2 Win\n", "→ Draw\n"]
        )
        with tempfile.TemporaryDirectory() as directory:
            result_path = Path(directory) / "gate_result.json"
            openings = Path(directory) / "openings.sfen"
            openings.write_text("startpos\n# ignored\n\n")
            argv = [
                "run_ab_match.py",
                "selfplay",
                "--engine-a", "candidate",
                "--engine-b", "baseline",
                "--games", "3",
                "--openings", str(openings),
                "--out-dir", directory,
                "--result-json", str(result_path),
            ]
            with patch.object(sys, "argv", argv), patch(
                "run_ab_match.subprocess.Popen", return_value=process
            ), patch(
                "run_ab_match.runner_identity", return_value={"commit": "test", "dirty": False}
            ), patch("sys.stdout", new_callable=io.StringIO):
                self.assertEqual(main(), 0)
            result = json.loads(result_path.read_text())

        self.assertEqual(result["terminal_state"], "completed")
        self.assertEqual(result["status"], "completed")
        self.assertEqual(result["elo"], result["result"]["elo"]["estimate"])
        self.assertEqual(result["ci"], result["result"]["elo"]["margin"])
        self.assertEqual(result["result"]["games_played"], 3)
        self.assertEqual(result["evidence"]["openings"]["position_count"], 1)
        self.assertIsNone(result["result"]["sprt"])

    def test_result_json_carries_validated_prospective_gate_declaration(self):
        process = _FakeMatchProcess(["→ Draw\n"])
        with tempfile.TemporaryDirectory() as directory:
            result_path = Path(directory) / "gate_result.json"
            declaration_path = Path(directory) / "declaration.json"
            declaration_path.write_text(
                json.dumps(
                    {
                        "schema": "sekirei.gate-observation-declaration.v1",
                        "candidate_id": "pilot-candidate",
                        "group_id": "pilot-independent-group",
                        "feature_schema_id": "sekirei.search-gate-features.v1",
                        "features": {"node_ratio": 1.25, "qsearch_share": 0.7},
                        "group_definition": {
                            "boundary": "same evaluator, openings, clock, and candidate family"
                        },
                        "engine_version": "0.3.68",
                        "source_commit": "deadbeef",
                        "evaluator": "material",
                        "time_control": {"byoyomi_ms": 100},
                        "threads": 1,
                        "opening_corpus": "openings_standard-v1",
                        "cost": {"games_limit": 1},
                    }
                ),
                encoding="utf-8",
            )
            declaration_input_sha256 = hashlib.sha256(declaration_path.read_bytes()).hexdigest()
            argv = [
                "run_ab_match.py",
                "selfplay",
                "--engine-a", "candidate",
                "--engine-b", "baseline",
                "--games", "1",
                "--out-dir", directory,
                "--result-json", str(result_path),
                "--gate-observation-declaration", str(declaration_path),
            ]
            with patch.object(sys, "argv", argv), patch(
                "run_ab_match.subprocess.Popen", return_value=process
            ), patch(
                "run_ab_match.runner_identity", return_value={"commit": "test", "dirty": False}
            ), patch("sys.stdout", new_callable=io.StringIO):
                self.assertEqual(main(), 0)
            result = json.loads(result_path.read_text())

        declaration = result["gate_observation"]
        self.assertEqual(declaration["candidate_id"], "pilot-candidate")
        self.assertEqual(len(declaration["feature_schema_sha256"]), 64)
        self.assertEqual(len(declaration["declaration_sha256"]), 64)
        self.assertEqual(
            result["evidence"]["gate_observation_declaration"]["sha256"],
            declaration_input_sha256,
        )

    def test_result_json_records_inconclusive_game_limit(self):
        process = _FakeMatchProcess(["→ Engine1 Win\n", "→ Engine2 Win\n"])
        with tempfile.TemporaryDirectory() as directory:
            result_path = Path(directory) / "gate_result.json"
            argv = [
                "run_ab_match.py",
                "selfplay",
                "--engine-a", "candidate",
                "--engine-b", "baseline",
                "--games", "2",
                "--sprt", "0,10",
                "--out-dir", directory,
                "--result-json", str(result_path),
            ]
            with patch.object(sys, "argv", argv), patch(
                "run_ab_match.subprocess.Popen", return_value=process
            ), patch(
                "run_ab_match.runner_identity", return_value={"commit": "test", "dirty": False}
            ), patch("sys.stdout", new_callable=io.StringIO):
                self.assertEqual(main(), 0)
            result = json.loads(result_path.read_text())

        self.assertEqual(result["terminal_state"], "inconclusive")
        self.assertEqual(result["status"], "inconclusive")
        self.assertEqual(result["result"]["sprt"]["verdict"], "inconclusive")

    def test_result_json_fails_closed_on_partial_process_failure(self):
        process = _FakeMatchProcess(["→ Engine1 Win\n"], exit_code=7)
        with tempfile.TemporaryDirectory() as directory:
            result_path = Path(directory) / "gate_result.json"
            argv = [
                "run_ab_match.py",
                "selfplay",
                "--engine-a", "candidate",
                "--engine-b", "baseline",
                "--games", "2",
                "--sprt", "0,10",
                "--out-dir", directory,
                "--result-json", str(result_path),
            ]
            with patch.object(sys, "argv", argv), patch(
                "run_ab_match.subprocess.Popen", return_value=process
            ), patch(
                "run_ab_match.runner_identity", return_value={"commit": "test", "dirty": False}
            ), patch("sys.stdout", new_callable=io.StringIO):
                self.assertEqual(main(), 7)
            result = json.loads(result_path.read_text())

        self.assertEqual(result["terminal_state"], "partial")
        self.assertEqual(result["status"], "error")
        self.assertFalse(result["evidence"]["complete"])
        self.assertEqual(result["evidence"]["process_exit_code"], 7)

    def test_result_json_fails_closed_when_process_cannot_start(self):
        with tempfile.TemporaryDirectory() as directory:
            result_path = Path(directory) / "gate_result.json"
            argv = [
                "run_ab_match.py",
                "selfplay",
                "--engine-a", "candidate",
                "--engine-b", "baseline",
                "--games", "1",
                "--out-dir", directory,
                "--result-json", str(result_path),
            ]
            with patch.object(sys, "argv", argv), patch(
                "run_ab_match.subprocess.Popen", side_effect=FileNotFoundError("missing")
            ), patch(
                "run_ab_match.runner_identity", return_value={"commit": "test", "dirty": False}
            ), patch("sys.stdout", new_callable=io.StringIO), patch(
                "sys.stderr", new_callable=io.StringIO
            ):
                self.assertEqual(main(), 1)
            result = json.loads(result_path.read_text())

        self.assertEqual(result["terminal_state"], "failed")
        self.assertEqual(result["status"], "error")
        self.assertIsNone(result["result"]["elo"]["margin"])
        self.assertIn("FileNotFoundError", result["evidence"]["process_error"])

    def test_result_json_cannot_overwrite_raw_match_output(self):
        with tempfile.TemporaryDirectory() as directory:
            argv = [
                "run_ab_match.py",
                "selfplay",
                "--engine-a", "candidate",
                "--engine-b", "baseline",
                "--games", "1",
                "--name", "collision",
                "--out-dir", directory,
                "--result-json", str(Path(directory) / "collision.json"),
            ]
            with patch.object(sys, "argv", argv), patch(
                "run_ab_match.subprocess.Popen"
            ) as popen, patch("sys.stderr", new_callable=io.StringIO):
                with self.assertRaises(SystemExit) as stopped:
                    main()

        self.assertEqual(stopped.exception.code, 2)
        popen.assert_not_called()

    def test_name_cannot_escape_output_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            argv = [
                "run_ab_match.py",
                "selfplay",
                "--engine-a", "candidate",
                "--engine-b", "baseline",
                "--games", "1",
                "--name", "../escape",
                "--out-dir", directory,
            ]
            with patch.object(sys, "argv", argv), patch(
                "run_ab_match.subprocess.Popen"
            ) as popen, patch("sys.stderr", new_callable=io.StringIO):
                with self.assertRaises(SystemExit) as stopped:
                    main()

        self.assertEqual(stopped.exception.code, 2)
        popen.assert_not_called()

    def test_malformed_openings_still_produce_failure_evidence(self):
        process = _FakeMatchProcess([], exit_code=2)
        with tempfile.TemporaryDirectory() as directory:
            result_path = Path(directory) / "gate_result.json"
            openings = Path(directory) / "openings.sfen"
            openings.write_bytes(b"\xff\n")
            argv = [
                "run_ab_match.py",
                "selfplay",
                "--engine-a", "candidate",
                "--engine-b", "baseline",
                "--games", "1",
                "--openings", str(openings),
                "--out-dir", directory,
                "--result-json", str(result_path),
            ]
            with patch.object(sys, "argv", argv), patch(
                "run_ab_match.subprocess.Popen", return_value=process
            ), patch(
                "run_ab_match.runner_identity", return_value={"commit": "test", "dirty": False}
            ), patch("sys.stdout", new_callable=io.StringIO):
                self.assertEqual(main(), 2)
            result = json.loads(result_path.read_text())

        self.assertEqual(result["terminal_state"], "failed")
        self.assertIn("UnicodeDecodeError", result["evidence"]["openings"]["position_count_error"])

    def test_modes_share_the_same_top_level_schema(self):
        schemas = []
        with tempfile.TemporaryDirectory() as directory:
            evalfile = Path(directory) / "nn.bin"
            evalfile.write_bytes(b"weights")
            for mode in ("selfplay", "yaneuraou"):
                result_path = Path(directory) / f"{mode}.json"
                argv = [
                    "run_ab_match.py",
                    mode,
                    "--engine-a", "candidate",
                    "--games", "1",
                    "--out-dir", directory,
                    "--result-json", str(result_path),
                ]
                if mode == "selfplay":
                    argv.extend(["--engine-b", "baseline"])
                else:
                    argv.extend(
                        ["--yaneuraou", "yaneuraou", "--evalfile", str(evalfile)]
                    )
                process = _FakeMatchProcess(["→ Draw\n"])
                with patch.object(sys, "argv", argv), patch(
                    "run_ab_match.subprocess.Popen", return_value=process
                ), patch(
                    "run_ab_match.runner_identity",
                    return_value={"commit": "test", "dirty": False},
                ), patch("sys.stdout", new_callable=io.StringIO):
                    self.assertEqual(main(), 0)
                schemas.append(set(json.loads(result_path.read_text())))

        self.assertEqual(schemas[0], schemas[1])


if __name__ == "__main__":
    unittest.main()
