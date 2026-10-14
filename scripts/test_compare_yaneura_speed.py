#!/usr/bin/env python3
"""Evidence validation and process isolation tests; no engines are invoked."""

import argparse
import json
import signal
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import compare_yaneura_speed as speed  # noqa: E402
import nps_threads  # noqa: E402


class SpeedComparisonTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        root = Path(self.temporary.name)
        self.args = argparse.Namespace(
            engine=root / "sekirei", reference=root / "YaneuraOu",
            evalfile=root / "nn.bin", positions=root / "positions.sfen",
            output_directory=root / "output", count=2, movetime=1000,
            threads="1", thread_counts=[1], timeout=30,
        )
        for name in ("engine", "reference", "evalfile"):
            getattr(self.args, name).write_bytes(name.encode())
        self.args.positions.write_text("startpos\nstartpos moves 7g7f\n")
        self.positions = nps_threads.read_positions(str(self.args.positions), 2)
        self.paths = {name: getattr(self.args, name)
                      for name in ("engine", "reference", "evalfile", "positions")}
        self.paths["driver"] = speed.DRIVER
        self.artifacts = speed.artifact_snapshot(self.paths)

    def rows(self, lane="sekirei", nps=(100, 200)):
        key = "engine" if lane == "sekirei" else "reference"
        return [{
            "schema": "sekirei.lazy-smp-scaling-row.v2",
            "position": position, "repeat": 0, "threads": 1,
            "nps": value, "depth": 5 + position, "nodes": value,
            "elapsed_ms": 1000, "bestmove": "7g7f",
            "provenance": {
                "engine": self.artifacts[key]["path"],
                "engine_sha256": self.artifacts[key]["sha256"],
                "positions": self.artifacts["positions"]["path"],
                "positions_file_sha256": self.artifacts["positions"]["sha256"],
                "effective_positions_sha256": nps_threads.effective_positions_sha256(self.positions),
                "effective_position_count": 2,
                "evaluator": {"kind": "halfkp", **self.artifacts["evalfile"]},
                "counter_overhead_included": False,
                "options": nps_threads.engine_options(speed.driver_args(self.args, lane), 1),
                "budget": "go movetime 1000",
            },
        } for position, value in enumerate(nps)]

    def validate(self, rows, lane="sekirei"):
        return speed.validate_rows(rows, self.args, lane, 1, self.artifacts, self.positions)

    def test_sekirei_budget_has_no_hidden_move_overhead(self):
        self.assertIn("MoveOverhead=0", speed.SEKIREI_OPTIONS)

    def test_complete_rows_are_sorted(self):
        self.assertEqual([row["position"] for row in self.validate(list(reversed(self.rows())))], [0, 1])
        self.validate(self.rows("reference"), "reference")

    def test_current_driver_fractional_elapsed_is_allowed_and_must_be_finite(self):
        rows = self.rows()
        rows[0]["elapsed_ms"] = 1000.123
        rows[1]["elapsed_ms"] = 0.125
        self.validate(rows)
        for value in (True, "1000", float("nan"), float("inf"), -0.1, 0.0):
            rows = self.rows()
            rows[0]["elapsed_ms"] = value
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, "elapsed_ms"):
                self.validate(rows)

    def test_current_driver_omits_only_unavailable_opening_book_option(self):
        rows = self.rows()
        for row in rows:
            row["provenance"]["opening_book_available"] = False
            row["provenance"]["options"].remove("UseBook=false")
        self.validate(rows)
        rows[0]["provenance"]["options"].remove("LazyDiagnostics=false")
        with self.assertRaisesRegex(ValueError, "options"):
            self.validate(rows)
        for available in (True, None, 0, "missing"):
            rows = self.rows()
            if available != "missing":
                rows[0]["provenance"]["opening_book_available"] = available
            rows[0]["provenance"]["options"].remove("UseBook=false")
            with self.subTest(available=available), self.assertRaisesRegex(ValueError, "options"):
                self.validate(rows)
        rows = self.rows("reference")
        rows[0]["provenance"]["opening_book_available"] = False
        rows[0]["provenance"]["options"].remove("USI_OwnBook=false")
        with self.assertRaisesRegex(ValueError, "options"):
            self.validate(rows, "reference")

    def test_incomplete_duplicate_or_out_of_range_positions_are_rejected(self):
        cases = [self.rows()[:1], self.rows() + self.rows()[:1]]
        for index in (-1, 2, True, 0):
            rows = self.rows()
            rows[1]["position"] = index
            cases.append(rows)
        for rows in cases:
            with self.subTest(rows=rows), self.assertRaises(ValueError):
                self.validate(rows)

    def test_mismatched_evidence_is_rejected(self):
        replacements = {
            "engine": "/other/engine", "engine_sha256": "different",
            "positions": "/other/corpus", "positions_file_sha256": "different",
            "effective_positions_sha256": "different", "effective_position_count": 1,
            "evaluator": {"kind": "material-only"}, "counter_overhead_included": True,
            "options": ["Threads=8"], "budget": "go depth 10",
        }
        for key, value in replacements.items():
            rows = self.rows()
            rows[1]["provenance"][key] = value
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, key):
                self.validate(rows)

    def test_missing_and_boolean_counter_evidence_is_rejected(self):
        for value in (None, 0, True):
            rows = self.rows()
            rows[0]["provenance"]["counter_overhead_included"] = value
            with self.subTest(value=value), self.assertRaises(ValueError):
                self.validate(rows)

    def test_diagnostic_rows_and_invalid_metrics_are_rejected(self):
        for key, value in (("lazy_smp", {}), ("lazy_smp_tt", {}),
                           ("lazy_smp_overlap", {}), ("lazy_smp_root_hint", {}),
                           ("nps", 0), ("nodes", -1),
                           ("depth", -1), ("elapsed_ms", 0), ("nps", float("inf")),
                           ("nps", True), ("repeat", 1), ("threads", 2), ("bestmove", "")):
            rows = self.rows()
            rows[0][key] = value
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                self.validate(rows)

    @staticmethod
    def production_lazy_summary():
        # Shape emitted by the real 2T run with LazyDiagnostics=false.
        return {
            "selected_worker": 1, "selected_node_share_permille": 502,
            "move_agreement": 2, "workers": 2,
            "discarded_root_nodes": 1060864, "divergent_discarded_nodes": 0,
            "stop_lag_ms": 0, "selected_tt_local_hits": 0,
            "selected_tt_foreign_hits": 0, "selected_tt_unknown_hits": 0,
            "worker_stats": [
                {"worker": worker, "selected": worker == 1,
                 "depth": 18 + worker, "nodes": nodes, "score": 4002,
                 "elapsed_ms": 951, "aborted": True, "abort_reason": "budget",
                 "tt_probes": 0, "tt_hits": 0, "tt_stores": 0,
                 "tt_local_hits": 0, "tt_foreign_hits": 0, "tt_unknown_hits": 0}
                for worker, nodes in enumerate((1060864, 1073152))
            ],
        }

    def two_thread_rows(self):
        rows = self.rows()
        for row in rows:
            row["threads"] = 2
            row["provenance"]["options"] = nps_threads.engine_options(
                speed.driver_args(self.args, "sekirei"), 2)
            row["lazy_smp"] = self.production_lazy_summary()
        return rows

    def test_uninstrumented_production_lazy_smp_summary_is_allowed(self):
        rows = self.two_thread_rows()
        result = speed.validate_rows(rows, self.args, "sekirei", 2,
                                     self.artifacts, self.positions)
        self.assertEqual(len(result), 2)
        self.assertIn("LazyDiagnostics=false", rows[0]["provenance"]["options"])

    def test_nonzero_counters_in_basic_lazy_smp_summary_are_rejected(self):
        for key in ("selected_tt_local_hits", "selected_tt_foreign_hits",
                    "selected_tt_unknown_hits", "tt_probes", "tt_hits",
                    "tt_stores", "tt_local_hits", "tt_foreign_hits", "tt_unknown_hits"):
            rows = self.two_thread_rows()
            summary = rows[0]["lazy_smp"]
            if key.startswith("selected_"):
                summary[key] = 1
            else:
                summary["worker_stats"][1][key] = 1
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, "counters"):
                speed.validate_rows(rows, self.args, "sekirei", 2,
                                    self.artifacts, self.positions)

    def test_geomean_uses_position_means_and_abba_order(self):
        values = [(100, 400), (100, 200), (300, 600), (100, 400)]
        arms = [{"index": index, "threads": 1, "lane": lane,
                 "status": "complete", "rows": self.rows(lane, value)}
                for index, (lane, value) in enumerate(zip(speed.ABBA, values))]
        result = speed.summarize(arms, 2)[0]
        self.assertAlmostEqual(result["geomean_sekirei_over_reference"], 2 ** 0.5)
        self.assertEqual(result["arms"][1]["median_nps"], 150)
        self.assertEqual(result["per_position_mean_nps_ratios"], [
            {"position": 0, "sekirei_over_reference": 2},
            {"position": 1, "sekirei_over_reference": 1},
        ])
        self.assertEqual(speed.summarize(arms[:3], 2), [])
        arms[2]["status"] = "failed"
        self.assertEqual(speed.summarize(arms, 2), [])
        arms[2]["status"] = "complete"
        arms[0], arms[1] = arms[1], arms[0]
        with self.assertRaises(ValueError):
            speed.summarize(arms, 2)

    def test_commands_use_same_budget_hash_and_evaluator(self):
        for lane in ("sekirei", "reference"):
            cmd = speed.command(self.args, lane, 4, Path("out.jsonl"))
            for option, value in (("--threads", "4"), ("--count", "2"),
                                  ("--movetime", "1000"), ("--hash", "64"),
                                  ("--fv-scale", "24"), ("--evalfile", str(self.args.evalfile))):
                self.assertEqual(cmd[cmd.index(option) + 1], value)
            self.assertNotIn("--lazy-diagnostics", cmd)
            self.assertEqual("--yaneuraou" in cmd, lane == "reference")
        self.assertIn("NnueOutput=absolute", speed.command(self.args, "sekirei", 1, Path("out")))
        self.assertIn("LazyDiagnostics=false", speed.command(self.args, "sekirei", 1, Path("out")))

    def test_timeout_kills_and_reaps_only_new_process_group_and_preserves_logs(self):
        stdout = Path(self.temporary.name) / "stdout.log"
        stderr = Path(self.temporary.name) / "stderr.log"
        with patch.object(speed.subprocess, "Popen") as popen, patch.object(speed.os, "killpg") as killpg:
            process = popen.return_value
            process.pid = 8123
            process.wait.side_effect = [subprocess.TimeoutExpired("driver", 5), -9]
            def start(*args, **kwargs):
                kwargs["stdout"].write(b"partial output\n")
                kwargs["stderr"].write(b"partial error\n")
                return process
            popen.side_effect = start
            with self.assertRaises(subprocess.TimeoutExpired):
                speed.run_bounded(["driver"], stdout, stderr, 5)
            self.assertTrue(popen.call_args.kwargs["start_new_session"])
            killpg.assert_called_once_with(8123, signal.SIGKILL)
            self.assertEqual(process.wait.call_count, 2)
        self.assertEqual(stdout.read_bytes(), b"partial output\n")
        self.assertEqual(stderr.read_bytes(), b"partial error\n")

    def run_mocked_main(self, mutation=None):
        def run(cmd, stdout, stderr, timeout):
            lane = "reference" if "--yaneuraou" in cmd else "sekirei"
            rows_path = Path(cmd[cmd.index("--json") + 1])
            rows_path.write_text("".join(json.dumps(row) + "\n" for row in self.rows(lane)))
            if mutation:
                mutation()
            return 0
        with patch.object(speed, "parse_args", return_value=self.args), patch.object(speed, "run_bounded", side_effect=run):
            return speed.main([])

    def test_main_saves_all_arms_in_abba_and_refuses_overwrite(self):
        self.assertEqual(self.run_mocked_main(), 0)
        report_path = self.args.output_directory / "report.json"
        original = report_path.read_bytes()
        report = json.loads(original)
        self.assertEqual(report["status"], "complete")
        self.assertEqual([arm["lane"] for arm in report["arms"]], list(speed.ABBA))
        self.assertEqual(report["artifacts_before"], report["artifacts_after"])
        with self.assertRaises(FileExistsError):
            self.run_mocked_main()
        self.assertEqual(report_path.read_bytes(), original)

    def test_artifact_mutation_preserves_failed_report_without_summary(self):
        with self.assertRaisesRegex(ValueError, "changed"):
            self.run_mocked_main(lambda: self.args.evalfile.write_bytes(b"changed"))
        report = json.loads((self.args.output_directory / "report.json").read_text())
        self.assertEqual(report["status"], "failed")
        self.assertEqual(report["arms"][0]["status"], "failed")
        self.assertNotEqual(report["arms"][0]["artifacts_before"], report["arms"][0]["artifacts_after"])
        self.assertEqual(report["summary"], [])

    def test_timeout_preserves_failed_report_and_completed_prior_arms(self):
        calls = 0

        def run(cmd, stdout, stderr, timeout):
            nonlocal calls
            calls += 1
            if calls == 2:
                raise subprocess.TimeoutExpired(cmd, timeout)
            rows_path = Path(cmd[cmd.index("--json") + 1])
            rows_path.write_text("".join(json.dumps(row) + "\n"
                                        for row in self.rows("reference")))
            return 0

        with patch.object(speed, "parse_args", return_value=self.args), patch.object(speed, "run_bounded", side_effect=run):
            with self.assertRaises(subprocess.TimeoutExpired):
                speed.main([])
        report = json.loads((self.args.output_directory / "report.json").read_text())
        self.assertEqual(report["status"], "failed")
        self.assertEqual([arm["status"] for arm in report["arms"]], ["complete", "failed"])
        self.assertEqual(report["summary"], [])
        self.assertIn("TimeoutExpired", report["error"])


if __name__ == "__main__":
    unittest.main()
