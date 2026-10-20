#!/usr/bin/env python3
"""Unit tests for the SPRT helpers in run_ab_match.py (stdlib unittest only).

Run: python3 scripts/test_run_ab_match.py
"""

import hashlib
import io
import json
import math
import signal
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import call, patch

sys.path.insert(0, str(Path(__file__).resolve().parent))

from run_ab_match import (  # noqa: E402
    classify_terminal_status,
    main,
    parse_args,
    paired_outcome,
    replay_durable_sprt,
    runner_identity,
    sekirei_options,
    sprt_bounds,
    sprt_llr,
    terminate_match_process_group,
    validate_candidate_material_evidence,
    validate_candidate_baseline_evidence,
    validate_raw_match_evidence,
    yaneuraou_options,
)


class _FakeMatchProcess:
    """Minimal stdlib-only `Popen` replacement for an early-stop regression."""

    def __init__(self, lines: list[str], exit_code: int = 0, before_yield=None):
        self._lines = iter(lines)
        self._exit_code = exit_code
        self._before_yield = before_yield
        self.stdout = self
        self.terminated = False
        self.lines_read = 0

    def __iter__(self):
        return self

    def __next__(self):
        line = next(self._lines)
        self.lines_read += 1
        if self._before_yield is not None:
            self._before_yield(self.lines_read, line)
        return line

    def terminate(self):
        self.terminated = True

    def poll(self):
        return 143 if self.terminated else None

    def wait(self):
        return -signal.SIGTERM if self.terminated else self._exit_code


class SprtTest(unittest.TestCase):
    def setUp(self):
        # Existing Popen unit tests use an in-memory fake process and therefore
        # do not produce sekirei-match artifacts. Raw-evidence behavior is
        # exercised directly below and once through main() with an injected
        # validation failure.
        self.raw_evidence_patcher = patch(
            "run_ab_match.validate_raw_match_evidence", return_value=None
        )
        self.raw_evidence = self.raw_evidence_patcher.start()
        self.replay_patcher = patch("run_ab_match.replay_durable_sprt")
        self.replay = self.replay_patcher.start()

    def tearDown(self):
        self.replay_patcher.stop()
        self.raw_evidence_patcher.stop()

    def test_terminal_status_keeps_error_precedence(self):
        status = classify_terminal_status(20, 20, 7, "OSError: broken pipe", "H1 accepted", True)
        self.assertEqual(status.terminal_state, "partial")
        self.assertEqual(status.gate_status, "error")
        self.assertEqual(status.exit_code, 7)

    def test_runner_identity_times_out_fail_closed(self):
        with patch(
            "run_ab_match.subprocess.run",
            side_effect=subprocess.TimeoutExpired(["git"], 5),
        ):
            identity = runner_identity()

        self.assertIsNone(identity["commit"])
        self.assertIsNone(identity["dirty"])

    def test_unexpected_process_failure_overrides_a_numerical_sprt_verdict(self):
        status = classify_terminal_status(
            20, 2000, 2, None, "H1 accepted", True, True
        )
        self.assertEqual(status.terminal_state, "partial")
        self.assertEqual(status.gate_status, "error")
        self.assertEqual(status.exit_code, 2)

    @unittest.skipUnless(sys.platform != "win32", "POSIX signal return code")
    def test_intentional_sprt_signal_exit_preserves_the_verdict(self):
        status = classify_terminal_status(
            20, 2000, -signal.SIGTERM, None, "H1 accepted", True, True
        )
        self.assertEqual(status.terminal_state, "sprt_stopped")
        self.assertEqual(status.gate_status, "pass")
        self.assertEqual(status.exit_code, 0)

    def test_early_stop_terminates_the_posix_process_group(self):
        process = _FakeMatchProcess([])
        process.pid = 4321
        with patch("run_ab_match.os.name", "posix"), patch(
            "run_ab_match.os.killpg", side_effect=[None, ProcessLookupError]
        ) as killpg:
            terminate_match_process_group(process)
        self.assertEqual(
            killpg.call_args_list,
            [call(4321, signal.SIGTERM), call(4321, 0)],
        )
        self.assertFalse(process.terminated)

    def test_early_stop_kills_process_group_survivors_after_grace(self):
        process = _FakeMatchProcess([])
        process.pid = 4321
        with patch("run_ab_match.os.name", "posix"), patch(
            "run_ab_match.os.killpg"
        ) as killpg:
            terminate_match_process_group(process, grace_seconds=0.0)
        self.assertEqual(
            killpg.call_args_list,
            [
                call(4321, signal.SIGTERM),
                call(4321, signal.SIGKILL),
            ],
        )
        self.assertFalse(process.terminated)

    def test_early_stop_falls_back_when_group_termination_fails(self):
        process = _FakeMatchProcess([])
        process.pid = 4321
        with patch("run_ab_match.os.name", "posix"), patch(
            "run_ab_match.os.killpg", side_effect=OSError("gone")
        ):
            terminate_match_process_group(process)
        self.assertTrue(process.terminated)

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

    def test_yaneuraou_minimum_thinking_time_uses_declared_lower_bound(self):
        # Pinned YaneuraOu c1b80e source/timeman.cpp declares
        # Option(2000, 1, 100000). Its parser silently ignores 0, so the
        # wrapper must request the valid lower bound rather than leave 2000.
        options = yaneuraou_options("weights.nnue", 24, 0)
        self.assertTrue(all(option == "--engine-option2" for option in options[::2]))
        values = [
            int(option.split("=", 1)[1])
            for option in options[1::2]
            if option.startswith("MinimumThinkingTime=")
        ]
        self.assertEqual(values, [1])

    def test_material_only_options_omit_evalfile_and_unadvertised_book_option(self):
        options = sekirei_options(
            "--engine-option1", None, 24, 1, "Speculative", 0, 64
        )
        self.assertNotIn("EvalFile=none", options)
        self.assertFalse(any("EvalFile=" in option for option in options))
        self.assertFalse(any("FV_SCALE=" in option for option in options))
        self.assertNotIn("UseBook=false", options)

    def test_candidate_material_evidence_requires_exact_asymmetric_weight_load(self):
        with tempfile.TemporaryDirectory() as directory:
            raw_path = Path(directory) / "match.json"
            candidate = str((Path(directory) / "nn_r6.bin").resolve())
            raw_path.write_text(
                json.dumps(
                    {
                        "engine1_options": {
                            "EvalFile": candidate,
                            "FV_SCALE": "24",
                            "NnueOutput": "absolute",
                        },
                        "engine2_options": {"Threads": "1"},
                        "engine1_eval_file_acknowledgement": (
                            f"info string NNUE weights loaded from {candidate}"
                        ),
                        "engine2_eval_file_acknowledgement": None,
                    }
                ),
                encoding="utf-8",
            )
            self.assertIsNone(validate_candidate_material_evidence(raw_path, candidate, 24))
            raw = json.loads(raw_path.read_text(encoding="utf-8"))
            raw["engine2_options"]["EvalFile"] = candidate
            raw_path.write_text(json.dumps(raw), encoding="utf-8")
            self.assertIn(
                "baseline unexpectedly",
                validate_candidate_material_evidence(raw_path, candidate, 24),
            )

    def test_candidate_material_gate_pins_only_engine_a_evaluator(self):
        process = _FakeMatchProcess(["→ Draw\n"])
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            candidate = root / "nn_r6.bin"
            engine = root / "sekirei"
            runner = root / "sekirei-match"
            openings = root / "openings.usi"
            for path, content in ((candidate, b"candidate"), (engine, b"engine"), (runner, b"runner")):
                path.write_bytes(content)
            openings.write_text("startpos\n", encoding="utf-8")
            snapshot = {
                "sha256": "1" * 64,
                "match_binary_sha256": hashlib.sha256(runner.read_bytes()).hexdigest(),
            }
            result_path = root / "result.json"
            argv = [
                "run_ab_match.py",
                "selfplay",
                "--engine-a", str(engine),
                "--engine-b", str(engine),
                "--match-binary", str(runner),
                "--openings", str(openings),
                "--openings-manifest", str(root / "manifest.json"),
                "--candidate-evalfile", str(candidate),
                "--games", "1",
                "--out-dir", str(root / "match-output"),
                "--result-json", str(result_path),
            ]
            with patch.object(sys, "argv", argv), patch(
                "run_ab_match.validate_corpus", side_effect=[snapshot, snapshot]
            ), patch(
                "run_ab_match.subprocess.Popen", return_value=process
            ) as popen, patch(
                "run_ab_match.validate_candidate_material_evidence", return_value=None
            ), patch(
                "run_ab_match.runner_identity", return_value={}
            ), patch("sys.stdout", new_callable=io.StringIO):
                self.assertEqual(main(), 0)
            result = json.loads(result_path.read_text())

        command = popen.call_args.args[0]
        engine1_options = [command[index + 1] for index, item in enumerate(command[:-1]) if item == "--engine-option1"]
        engine2_options = [command[index + 1] for index, item in enumerate(command[:-1]) if item == "--engine-option2"]
        self.assertIn(f"EvalFile={candidate.resolve()}", engine1_options)
        self.assertIn("FV_SCALE=24", engine1_options)
        self.assertIn("NnueOutput=absolute", engine1_options)
        self.assertFalse(any(option.startswith(("EvalFile=", "FV_SCALE=", "NnueOutput=")) for option in engine2_options))
        self.assertEqual(result["evidence"]["evaluation"]["mode"], "candidate-halfkp-vs-material")

    def test_candidate_material_gate_rejects_evaluator_side_overrides(self):
        argv = [
            "run_ab_match.py",
            "selfplay",
            "--engine-a", "candidate",
            "--engine-b", "baseline",
            "--candidate-evalfile", "candidate.bin",
            "--openings-manifest", "openings.json",
            "--option-a", "NnueOutput=residual-material",
        ]
        with patch.object(sys, "argv", argv), patch("sys.stderr", new_callable=io.StringIO):
            with self.assertRaises(SystemExit) as stopped:
                main()
        self.assertEqual(stopped.exception.code, 2)

    def run_dual_gate_fixture(self, directory, *, material=False, mutate_baseline=False,
                              tampered_pairs=False, matching_pairs=False, different_engine=False):
        root = Path(directory)
        candidate, baseline, engine, runner = [root / name for name in
                                               ("candidate.bin", "baseline.bin", "engine", "match")]
        for path, content in ((candidate, b"candidate"), (baseline, b"baseline"),
                              (engine, b"engine"), (runner, b"runner")):
            path.write_bytes(content)
        engine_b = engine
        if different_engine:
            engine_b = root / "other-engine"
            engine_b.write_bytes(b"different binary")
        openings = root / "openings.usi"
        openings.write_text("startpos\n", encoding="utf-8")
        out = root / "output"
        out.mkdir()
        candidate_options = {"EvalFile": str(candidate.resolve()), "FV_SCALE": "24",
                             "NnueOutput": "absolute"}
        baseline_options = {} if material else dict(candidate_options, EvalFile=str(baseline.resolve()))
        raw = {"engine1_options": candidate_options, "engine2_options": baseline_options,
               "engine1_command": str(engine), "engine2_command": str(engine_b),
               "engine1_eval_file_acknowledgement": f"info string NNUE weights loaded from {candidate.resolve()}",
               "engine2_eval_file_acknowledgement": None if material else f"info string NNUE weights loaded from {baseline.resolve()}"}
        (out / "ab.json").write_text(json.dumps(raw))
        if tampered_pairs:
            # Same aggregate scores, different colour-pair assignments.
            outcomes = (["candidate_win", "candidate_win", "baseline_win", "baseline_win"] if matching_pairs
                        else ["candidate_win", "baseline_win", "candidate_win", "baseline_win"])
            (out / "ab.jsonl").write_text("".join(json.dumps({"id": f"pos{i//2}_pair0", "result": x}) + "\n"
                                                    for i, x in enumerate(outcomes)))
        argv = ["run_ab_match.py", "selfplay", "--engine-a", str(engine), "--engine-b", str(engine_b),
                "--match-binary", str(runner), "--candidate-evalfile", str(candidate),
                "--openings", str(openings), "--openings-manifest", str(root / "manifest.json"),
                "--out-dir", str(out), "--result-json", str(root / "result.json"), "--games", "4" if tampered_pairs else "2"]
        if not material:
            argv += ["--baseline-evalfile", str(baseline)]
        if tampered_pairs:
            argv += ["--sprt", "0,20", "--paired-sprt", "--games-per-position", "4"]
        process = _FakeMatchProcess(
            ["→ Engine1 Win\n", "→ Engine1 Win\n", "→ Engine2 Win\n", "→ Engine2 Win\n"]
            if tampered_pairs else ["→ Draw\n"] * 2,
            before_yield=(lambda *_: baseline.write_bytes(b"changed")) if mutate_baseline else None)
        snapshot = {"sha256": "1" * 64,
                    "match_binary_sha256": hashlib.sha256(runner.read_bytes()).hexdigest()}
        with patch.object(sys, "argv", argv), patch("run_ab_match.validate_corpus", return_value=snapshot), \
             patch("run_ab_match.subprocess.Popen", return_value=process) as popen, \
             patch("run_ab_match.runner_identity", return_value={}), patch("sys.stdout", new_callable=io.StringIO):
            code = main()
        return code, json.loads((root / "result.json").read_text()) if (root / "result.json").exists() else None, popen

    def test_dual_halfkp_gate_binds_both_weights_and_absolute_modes(self):
        with tempfile.TemporaryDirectory() as directory:
            code, result, popen = self.run_dual_gate_fixture(directory)
            self.assertEqual(code, 0)
            command = popen.call_args.args[0]
            for side, name in ((1, "candidate.bin"), (2, "baseline.bin")):
                options = [command[i+1] for i, value in enumerate(command[:-1]) if value == f"--engine-option{side}"]
                self.assertIn(f"EvalFile={(Path(directory)/name).resolve()}", options)
                self.assertIn("FV_SCALE=24", options)
                self.assertIn("NnueOutput=absolute", options)
            evaluation = result["evidence"]["evaluation"]
            self.assertEqual(evaluation["mode"], "candidate-halfkp-vs-halfkp")
            self.assertEqual(evaluation["baseline"], "halfkp")
            self.assertEqual(evaluation["baseline_file"]["sha256"], hashlib.sha256(b"baseline").hexdigest())
            pinned = result["evidence"]["pinned_inputs_before_match"]
            self.assertIn(evaluation["baseline_file"], pinned)

    def test_dual_halfkp_gate_rejects_baseline_mutation(self):
        with tempfile.TemporaryDirectory() as directory:
            code, result, _ = self.run_dual_gate_fixture(directory, mutate_baseline=True)
            self.assertNotEqual(code, 0)
            self.assertEqual(result["status"], "error")
            self.assertIn("inputs changed", result["evidence"]["process_error"])

    def test_dual_halfkp_gate_requires_same_fixed_engine_binary(self):
        with tempfile.TemporaryDirectory() as directory, patch("sys.stderr", new_callable=io.StringIO):
            code, result, popen = self.run_dual_gate_fixture(directory, different_engine=True)
            self.assertEqual(code, 2)
            self.assertIsNone(result)
            popen.assert_not_called()

    def test_evaluator_gates_also_replay_durable_paired_sprt(self):
        self.replay.side_effect = replay_durable_sprt
        for material in (False, True):
            with self.subTest(material=material), tempfile.TemporaryDirectory() as directory:
                code, result, _ = self.run_dual_gate_fixture(directory, material=material, tampered_pairs=True)
                self.assertNotEqual(code, 0)
                self.assertEqual(result["status"], "error")
                self.assertIn("durable SPRT replay disagrees", result["evidence"]["process_error"])
        self.assertEqual(self.replay.call_count, 2)

    def test_evaluator_gates_accept_matching_durable_paired_sprt(self):
        self.replay.side_effect = replay_durable_sprt
        for material in (False, True):
            with self.subTest(material=material), tempfile.TemporaryDirectory() as directory:
                code, result, _ = self.run_dual_gate_fixture(directory, material=material, tampered_pairs=True, matching_pairs=True)
                self.assertEqual(code, 0)
                self.assertEqual(result["status"], "inconclusive")
                self.assertIsNone(result["evidence"]["process_error"])
        self.assertEqual(self.replay.call_count, 2)

    def test_dual_halfkp_evidence_rejects_each_wrong_arm_option_or_ack(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "raw.json"
            weights = ("/candidate.bin", "/baseline.bin")
            raw = {}
            for side, weight in enumerate(weights, 1):
                raw[f"engine{side}_options"] = {"EvalFile": weight, "FV_SCALE": "24", "NnueOutput": "absolute"}
                raw[f"engine{side}_eval_file_acknowledgement"] = f"info string NNUE weights loaded from {weight}"
                raw[f"engine{side}_command"] = "/engine"
            path.write_text(json.dumps(raw))
            self.assertIsNone(validate_candidate_baseline_evidence(path, *weights, 24, ("/engine", "/engine")))
            for side in (1, 2):
                for field, value in (("EvalFile", weights[2-side]), ("FV_SCALE", "16"), ("NnueOutput", "residual-material")):
                    altered = json.loads(json.dumps(raw))
                    altered[f"engine{side}_options"][field] = value
                    path.write_text(json.dumps(altered))
                    self.assertIsNotNone(validate_candidate_baseline_evidence(path, *weights, 24))
                for field in (f"engine{side}_eval_file_acknowledgement", f"engine{side}_command"):
                    altered = dict(raw, **{field: None})
                    path.write_text(json.dumps(altered))
                    self.assertIsNotNone(validate_candidate_baseline_evidence(path, *weights, 24, ("/engine", "/engine")))

    def test_dual_halfkp_cli_rejects_invalid_combinations_and_side_overrides(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            candidate, baseline = root / "candidate", root / "baseline"
            candidate.write_bytes(b"candidate"); baseline.write_bytes(b"baseline")
            required = ["selfplay", "--engine-a", "engine", "--engine-b", "engine", "--candidate-evalfile", str(candidate),
                        "--baseline-evalfile", str(baseline), "--openings-manifest", "manifest", "--result-json", str(root / "result")]
            cases = [
                ["selfplay", "--engine-a", "engine", "--engine-b", "engine", "--baseline-evalfile", str(baseline)],
                ["yaneuraou", "--engine-a", "engine", "--yaneuraou", "engine", "--evalfile", "shared", "--candidate-evalfile", str(candidate), "--baseline-evalfile", str(baseline)],
                required + ["--evalfile", "shared"],
                required[:required.index("--openings-manifest")] + ["--result-json", str(root / "result")],
                required + ["--baseline-evalfile", str(root / "missing")],
                required + ["--candidate-evalfile", str(root / "missing")],
                required + ["--baseline-evalfile", str(candidate)],
            ]
            clone = root / "same-content"; clone.write_bytes(candidate.read_bytes())
            cases.append(required + ["--baseline-evalfile", str(clone)])
            for side in ("a", "b"):
                for option in ("EvalFile=other", "FV_SCALE=16", "NnueOutput=residual-material"):
                    cases.append(required + [f"--option-{side}", option])
            for argv in cases:
                with self.subTest(argv=argv), patch("sys.stderr", new_callable=io.StringIO), self.assertRaises(SystemExit) as error:
                    parse_args(argv)
                self.assertEqual(error.exception.code, 2)

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

    def test_trinomial_llr_reduces_to_wald_without_draws(self):
        wins, losses = 700, 300
        p0 = 1 / (1 + 10 ** (0 / 400))
        p1 = 1 / (1 + 10 ** (-10 / 400))
        expected = wins * math.log(p1 / p0) + losses * math.log((1 - p1) / (1 - p0))
        self.assertAlmostEqual(sprt_llr(wins, losses, 0, 0, 10), expected, places=9)

    def test_draw_heavy_llr_matches_veridict_reference(self):
        # veridict 0.19.5, trinomial BayesElo [0, 10], W/D/L = 500/300/200.
        self.assertAlmostEqual(sprt_llr(500, 200, 300, 0, 10), 11.0336196488132, places=12)

    def test_paired_outcome_uses_net_two_game_score(self):
        self.assertEqual(paired_outcome(1.0, 1.0), "win")
        self.assertEqual(paired_outcome(1.0, 0.0), "draw")
        self.assertEqual(paired_outcome(0.5, 0.5), "draw")
        self.assertEqual(paired_outcome(0.0, 0.0), "loss")

    def test_main_terminates_match_when_sprt_accepts_h1(self):
        # The process emits a result stream which would accept H1 well before
        # its configured 600-game upper bound.  No engine binary is needed.
        lines = (
            ["→ Engine1 Win\n"] * 300
            + ["→ Engine2 Win\n"] * 180
            + ["→ Draw\n"] * 120
        )
        process = _FakeMatchProcess(lines)
        self.replay.side_effect = lambda _path, *, sprt, paired: (
            (process.lines_read, 0, 0),
            sprt_llr(process.lines_read, 0, 0, sprt[0], sprt[1]),
            "H1 accepted",
        )
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

    def test_raw_match_evidence_requires_complete_matching_artifacts(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            raw_json = root / "match.json"
            kifu_dir = root / "kifu_match"
            kifu_dir.mkdir()
            (kifu_dir / "game0001.txt").write_text("game one", encoding="utf-8")
            raw_json.with_suffix(".jsonl").write_text(
                '{"id":"game0001","result":"candidate_win"}\n', encoding="utf-8"
            )
            clean = {
                "status": "complete",
                "games": 1,
                "engine1_wins": 1,
                "engine2_wins": 0,
                "draws": 0,
                "artifact_files_expected": 1,
                "artifact_write_failures": [],
                "invalid_games": [],
            }
            raw_json.write_text(json.dumps(clean), encoding="utf-8")
            self.assertIsNone(
                validate_raw_match_evidence(
                    raw_json,
                    kifu_dir,
                    games_played=1,
                    wins=1,
                    losses=0,
                    draws=0,
                    allow_running_snapshot=False,
                )
            )

            raw_json.with_suffix(".jsonl").write_text(
                '{"id":"game0001","result":"baseline_win"}\n', encoding="utf-8"
            )
            self.assertIn(
                "JSONL outcomes",
                validate_raw_match_evidence(
                    raw_json,
                    kifu_dir,
                    games_played=1,
                    wins=1,
                    losses=0,
                    draws=0,
                    allow_running_snapshot=False,
                ),
            )
            raw_json.with_suffix(".jsonl").write_text(
                '{"id":"game0001","result":"candidate_win"}\n', encoding="utf-8"
            )

            cases = {
                "running normal completion": {**clean, "status": "running"},
                "artifact failure": {
                    **clean,
                    "artifact_write_failures": ["game0001.txt"],
                },
                "invalid game": {**clean, "invalid_games": ["game0001: TimeForfeit"]},
                "count mismatch": {**clean, "games": 2},
            }
            for label, payload in cases.items():
                with self.subTest(label=label):
                    raw_json.write_text(json.dumps(payload), encoding="utf-8")
                    self.assertIsNotNone(
                        validate_raw_match_evidence(
                            raw_json,
                            kifu_dir,
                            games_played=1,
                            wins=1,
                            losses=0,
                            draws=0,
                            allow_running_snapshot=False,
                        )
                    )

            raw_json.write_text(json.dumps({**clean, "status": "running"}), encoding="utf-8")
            self.assertIsNone(
                validate_raw_match_evidence(
                    raw_json,
                    kifu_dir,
                    games_played=1,
                    wins=1,
                    losses=0,
                    draws=0,
                    allow_running_snapshot=True,
                )
            )
            (kifu_dir / "game0001.txt").unlink()
            self.assertIn(
                "kifu artifact set mismatch",
                validate_raw_match_evidence(
                    raw_json,
                    kifu_dir,
                    games_played=1,
                    wins=1,
                    losses=0,
                    draws=0,
                    allow_running_snapshot=True,
                ),
            )
            (kifu_dir / "game0001.txt").write_text("", encoding="utf-8")
            self.assertIn(
                "is empty",
                validate_raw_match_evidence(
                    raw_json,
                    kifu_dir,
                    games_played=1,
                    wins=1,
                    losses=0,
                    draws=0,
                    allow_running_snapshot=True,
                ),
            )

    def test_durable_paired_sprt_replay_uses_record_order(self):
        with tempfile.TemporaryDirectory() as directory:
            raw_json = Path(directory) / "paired.json"
            records = [
                {"id": "pair0", "result": "candidate_win"},
                {"id": "pair0", "result": "candidate_win"},
                {"id": "pair1", "result": "baseline_win"},
                {"id": "pair1", "result": "baseline_win"},
            ]
            raw_json.with_suffix(".jsonl").write_text(
                "".join(json.dumps(record) + "\n" for record in records),
                encoding="utf-8",
            )
            sprt = (0.0, 10.0, *sprt_bounds())

            counts, _, _ = replay_durable_sprt(raw_json, sprt=sprt, paired=True)

            reordered = [records[0], records[2], records[1], records[3]]
            raw_json.with_suffix(".jsonl").write_text(
                "".join(json.dumps(record) + "\n" for record in reordered),
                encoding="utf-8",
            )
            reordered_counts, _, _ = replay_durable_sprt(
                raw_json, sprt=sprt, paired=True
            )

        self.assertEqual(counts, (1, 1, 0))
        # The same aggregate game totals in W/L/W/L order are two pair draws.
        self.assertEqual(reordered_counts, (0, 0, 2))

    def test_h1_acceptance_uses_the_real_durable_evidence_validator(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            raw_json = root / "durable.json"
            raw_jsonl = raw_json.with_suffix(".jsonl")
            kifu_dir = root / "kifu_durable"
            kifu_dir.mkdir()
            records = []

            def persist_before_result(index, _line):
                (kifu_dir / f"game{index:04}.txt").write_text(
                    f"game {index}", encoding="utf-8"
                )
                records.append(
                    json.dumps({"id": f"game{index:04}", "result": "candidate_win"})
                )
                raw_jsonl.write_text("\n".join(records) + "\n", encoding="utf-8")
                raw_json.write_text(
                    json.dumps(
                        {
                            "status": "running",
                            "games": index,
                            "engine1_wins": index,
                            "engine2_wins": 0,
                            "draws": 0,
                            "artifact_files_expected": index,
                            "artifact_write_failures": [],
                            "invalid_games": [],
                        }
                    ),
                    encoding="utf-8",
                )

            process = _FakeMatchProcess(
                ["→ Engine1 Win\n"] * 600,
                before_yield=persist_before_result,
            )
            result_path = root / "gate_result.json"
            argv = [
                "run_ab_match.py",
                "selfplay",
                "--engine-a", "candidate",
                "--engine-b", "baseline",
                "--games", "600",
                "--sprt", "0,10",
                "--name", "durable",
                "--out-dir", directory,
                "--result-json", str(result_path),
            ]
            with patch.object(sys, "argv", argv), patch(
                "run_ab_match.subprocess.Popen", return_value=process
            ), patch(
                "run_ab_match.runner_identity", return_value={"commit": "test", "dirty": False}
            ), patch(
                "run_ab_match.validate_raw_match_evidence",
                wraps=validate_raw_match_evidence,
            ), patch(
                "run_ab_match.replay_durable_sprt",
                wraps=replay_durable_sprt,
            ), patch("sys.stdout", new_callable=io.StringIO):
                self.assertEqual(main(), 0)
            result = json.loads(result_path.read_text())

        self.assertTrue(process.terminated)
        self.assertEqual(result["status"], "pass")
        self.assertEqual(result["result"]["sprt"]["verdict"], "accept_h1")

    def test_independent_input_mutation_revokes_an_otherwise_accepted_h1(self):
        for mutation in ("manifest", "engine"):
            with self.subTest(mutation=mutation), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                engine = root / "engine"
                runner = root / "match"
                openings = root / "openings.usi"
                engine.write_bytes(b"frozen-engine")
                runner.write_bytes(b"frozen-runner")
                openings.write_text("startpos\n")
                snapshot = {"sha256": "1" * 64, "match_binary_sha256": hashlib.sha256(runner.read_bytes()).hexdigest()}
                changed = dict(snapshot, sha256="2" * 64)

                def mutate_engine(index, _line):
                    if mutation == "engine" and index == 1:
                        engine.write_bytes(b"changed-engine")

                process = _FakeMatchProcess(["→ Engine1 Win\n"] * 600, before_yield=mutate_engine)
                self.replay.side_effect = lambda _path, *, sprt, paired: (
                    (process.lines_read, 0, 0),
                    sprt_llr(process.lines_read, 0, 0, sprt[0], sprt[1]),
                    "H1 accepted",
                )
                result_path = root / "result.json"
                argv = ["run_ab_match.py", "selfplay", "--engine-a", str(engine), "--engine-b", str(engine),
                        "--match-binary", str(runner), "--openings", str(openings),
                        "--openings-manifest", str(root / "manifest.json"),
                        "--games", "600", "--sprt", "0,10", "--out-dir", str(root / "match-output"),
                        "--result-json", str(result_path)]
                with patch.object(sys, "argv", argv), patch("run_ab_match.validate_corpus", side_effect=[snapshot, changed if mutation == "manifest" else snapshot]), patch("run_ab_match.subprocess.Popen", return_value=process), patch("run_ab_match.runner_identity", return_value={}), patch("sys.stdout", new_callable=io.StringIO), patch("sys.stderr", new_callable=io.StringIO):
                    self.assertEqual(main(), 1)
                result = json.loads(result_path.read_text())
                self.assertTrue(process.terminated)
                self.assertEqual(result["status"], "error")
                self.assertEqual(result["result"]["sprt"]["verdict"], "inconclusive")
                self.assertFalse(result["evidence"]["complete"])
                self.assertIn("inputs changed", result["evidence"]["process_error"])
                self.assertEqual(result["evidence"]["openings"]["independent_corpus"], snapshot)

    def test_result_json_fails_closed_when_raw_evidence_is_invalid(self):
        process = _FakeMatchProcess(
            ["→ Engine1 Win\n"] * 300
            + ["→ Engine2 Win\n"] * 180
            + ["→ Draw\n"] * 120
        )
        self.raw_evidence.return_value = "raw match reports invalid games"
        with tempfile.TemporaryDirectory() as directory:
            result_path = Path(directory) / "gate_result.json"
            argv = [
                "run_ab_match.py",
                "selfplay",
                "--engine-a", "candidate",
                "--engine-b", "baseline",
                "--games", "600",
                "--sprt", "0,10",
                "--out-dir", directory,
                "--result-json", str(result_path),
            ]
            with patch.object(sys, "argv", argv), patch(
                "run_ab_match.subprocess.Popen", return_value=process
            ), patch(
                "run_ab_match.runner_identity", return_value={"commit": "test", "dirty": False}
            ), patch("sys.stdout", new_callable=io.StringIO), patch(
                "sys.stderr", new_callable=io.StringIO
            ):
                self.assertEqual(main(), 1)
            result = json.loads(result_path.read_text())

        self.assertEqual(result["status"], "error")
        self.assertEqual(result["terminal_state"], "partial")
        self.assertEqual(result["result"]["sprt"]["verdict"], "inconclusive")
        self.assertFalse(result["evidence"]["complete"])
        self.assertIn("EvidenceError", result["evidence"]["process_error"])

    def test_paired_sprt_stops_only_after_a_complete_pair(self):
        lines = ["→ Engine1 Win\n"] * 600
        process = _FakeMatchProcess(lines)
        self.replay.side_effect = lambda _path, *, sprt, paired: (
            (process.lines_read // 2, 0, 0),
            sprt_llr(process.lines_read // 2, 0, 0, sprt[0], sprt[1]),
            "H1 accepted",
        )
        with tempfile.TemporaryDirectory() as directory:
            openings = Path(directory) / "openings.sfen"
            openings.write_text("startpos\n", encoding="utf-8")
            result_path = Path(directory) / "paired_result.json"
            argv = [
                "run_ab_match.py",
                "selfplay",
                "--engine-a", "candidate",
                "--engine-b", "baseline",
                "--games", "600",
                "--games-per-position", "600",
                "--paired-sprt",
                "--sprt", "0,10",
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

        self.assertTrue(process.terminated)
        self.assertEqual(process.lines_read % 2, 0)
        self.assertTrue(result["result"]["sprt"]["paired_by_position"])
        observations = result["result"]["sprt"]["observations"]
        self.assertEqual(observations["wins"], process.lines_read // 2)
        self.assertEqual(observations["losses"], 0)
        self.assertEqual(observations["draws"], 0)

    def test_games_per_position_cannot_exceed_games_hard_cap(self):
        with tempfile.TemporaryDirectory() as directory:
            openings = Path(directory) / "openings.sfen"
            openings.write_text("startpos\nstartpos moves 7g7f\n", encoding="utf-8")
            argv = [
                "run_ab_match.py",
                "selfplay",
                "--engine-a", "candidate",
                "--engine-b", "baseline",
                "--games", "2",
                "--games-per-position", "2",
                "--openings", str(openings),
                "--out-dir", directory,
            ]
            with patch.object(sys, "argv", argv), patch(
                "run_ab_match.subprocess.Popen"
            ) as popen, patch("sys.stderr", new_callable=io.StringIO) as stderr:
                self.assertEqual(main(), 2)

        popen.assert_not_called()
        self.assertIn("--games hard cap", stderr.getvalue())

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
        self.replay.return_value = ((1, 1, 0), sprt_llr(1, 1, 0, 0, 10), "")
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
        self.replay.return_value = ((1, 0, 0), sprt_llr(1, 0, 0, 0, 10), "")
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

    def test_unexpected_exit_clears_a_streamed_h1_verdict(self):
        process = _FakeMatchProcess(["→ Engine1 Win\n"] * 600)
        process.wait = lambda: 7
        self.replay.side_effect = lambda _path, *, sprt, paired: (
            (process.lines_read, 0, 0),
            sprt_llr(process.lines_read, 0, 0, sprt[0], sprt[1]),
            "H1 accepted",
        )
        with tempfile.TemporaryDirectory() as directory:
            result_path = Path(directory) / "failed_after_boundary.json"
            argv = [
                "run_ab_match.py",
                "selfplay",
                "--engine-a", "candidate",
                "--engine-b", "baseline",
                "--games", "600",
                "--sprt", "0,10",
                "--out-dir", directory,
                "--result-json", str(result_path),
            ]
            with patch.object(sys, "argv", argv), patch(
                "run_ab_match.subprocess.Popen", return_value=process
            ), patch(
                "run_ab_match.runner_identity", return_value={"commit": "test", "dirty": False}
            ), patch("sys.stdout", new_callable=io.StringIO), patch(
                "sys.stderr", new_callable=io.StringIO
            ):
                self.assertEqual(main(), 7)
            result = json.loads(result_path.read_text())

        self.assertEqual(result["status"], "error")
        self.assertEqual(result["result"]["sprt"]["verdict"], "inconclusive")
        self.assertIn("unexpectedly", result["evidence"]["process_error"])

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
            engines = [Path(directory) / name for name in ("candidate", "yaneuraou", "match")]
            for engine in engines:
                engine.write_bytes(b"synthetic executable identity")
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
                        ["--yaneuraou", str(engines[1]), "--evalfile", str(evalfile),
                         "--match-binary", str(engines[2])]
                    )
                if mode == "yaneuraou":
                    argv[argv.index("--engine-a") + 1] = str(engines[0])
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


    @staticmethod
    def yane_fixture(directory):
        root = Path(directory)
        paths = {key: root / name for key, name in (
            ("requested", "selected.bin"), ("native", "nn.bin"),
            ("engine_a", "sekirei"), ("engine_b", "YaneuraOu"), ("runner", "match"))}
        for key, path in paths.items():
            path.write_bytes(b"synthetic-network" if key in {"requested", "native"} else key.encode())
        argv = ["run_ab_match.py", "yaneuraou", "--engine-a", str(paths["engine_a"]),
                "--yaneuraou", str(paths["engine_b"]), "--match-binary", str(paths["runner"]),
                "--evalfile", str(paths["requested"]), "--games", "1",
                "--out-dir", str(root / "output"), "--result-json", str(root / "output" / "result.json")]
        return paths, argv

    def test_yaneuraou_mismatched_native_evaluator_rejected_before_launch(self):
        with tempfile.TemporaryDirectory() as directory:
            paths, argv = self.yane_fixture(directory)
            paths["native"].write_bytes(b"other-network")
            with patch.object(sys, "argv", argv), patch("run_ab_match.subprocess.Popen") as popen, patch(
                "sys.stderr", new_callable=io.StringIO
            ) as errors:
                self.assertEqual(main(), 2)
            popen.assert_not_called()
            self.assertIn("differs", errors.getvalue())
            self.assertFalse((Path(directory) / "output").exists())

    def test_yaneuraou_protected_overrides_rejected_before_launch(self):
        for option in (" EvalDir =other", "EvalFile=other", "FV_SCALE=16", "NnueOutput=relative",
                       "Threads=1\nsetoption name EvalDir value other",
                       "FV_SCALE value 16=ignored", "EvalDir ignored /other=ignored"):
            with self.subTest(option=option), tempfile.TemporaryDirectory() as directory:
                _, argv = self.yane_fixture(directory)
                argv.extend(["--option-b", option])
                with patch.object(sys, "argv", argv), patch("run_ab_match.subprocess.Popen") as popen, patch(
                    "sys.stderr", new_callable=io.StringIO
                ):
                    self.assertEqual(main(), 2)
                popen.assert_not_called()
                self.assertFalse((Path(directory) / "output").exists())

    def test_yaneuraou_equal_renamed_file_binds_actual_native_and_executables(self):
        with tempfile.TemporaryDirectory() as directory:
            paths, argv = self.yane_fixture(directory)
            with patch.object(sys, "argv", argv), patch(
                "run_ab_match.subprocess.Popen", return_value=_FakeMatchProcess(["→ Draw\n"])
            ) as popen, patch("run_ab_match.runner_identity", return_value={}), patch(
                "sys.stdout", new_callable=io.StringIO
            ):
                self.assertEqual(main(), 0)
            payload = json.loads((Path(directory) / "output" / "result.json").read_text())
            binding = payload["evidence"]["evaluation"]["native_yaneuraou_binding"]
            self.assertEqual(binding["requested"]["sha256"], binding["native_nnue"]["sha256"])
            self.assertEqual(binding["native_nnue"]["path"], str(paths["native"]))
            self.assertEqual(set(payload["evidence"]["yaneuraou_pinned_inputs_before_match"]),
                             {"engine_a", "yaneuraou", "match_binary", "wrapper", "artifact_helper", "evaluator_binding_helper"})
            command = popen.call_args.args[0]
            self.assertIn("EvalFile=" + str(paths["requested"]), command)
            self.assertIn("EvalDir=" + str(paths["native"].parent), command)
            self.assertEqual(payload["terminal_state"], "completed")

    def test_yaneuraou_input_mutation_revokes_a_numerical_h1(self):
        for changed in ("requested", "native", "engine_a", "engine_b", "runner"):
            with self.subTest(changed=changed), tempfile.TemporaryDirectory() as directory:
                paths, argv = self.yane_fixture(directory)
                argv.extend(["--sprt", "0,10"])
                process = _FakeMatchProcess(["→ Engine1 Win\n"],
                    before_yield=lambda _index, _line: paths[changed].write_bytes(b"changed"))
                self.replay.return_value = ((1, 0, 0), 4.0, "H1 accepted")
                with patch.object(sys, "argv", argv), patch("run_ab_match.subprocess.Popen", return_value=process), patch(
                    "run_ab_match.runner_identity", return_value={}
                ), patch("run_ab_match.sprt_llr", return_value=4.0), patch(
                    "sys.stdout", new_callable=io.StringIO
                ), patch("sys.stderr", new_callable=io.StringIO):
                    self.assertNotEqual(main(), 0)
                payload = json.loads((Path(directory) / "output" / "result.json").read_text())
                self.assertEqual(payload["status"], "error")
                self.assertFalse(payload["evidence"]["complete"])
                self.assertNotEqual(payload["result"]["sprt"]["verdict"], "accept_h1")
                self.assertIn("input drift", payload["evidence"]["process_error"])


if __name__ == "__main__":
    unittest.main()
