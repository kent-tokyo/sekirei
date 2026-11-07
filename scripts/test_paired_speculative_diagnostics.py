#!/usr/bin/env python3
"""Synthetic contracts only: these tests never start a shogi engine."""
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import paired_speculative_diagnostics as driver
import validate_paired_pv as pv_validator


IDLE = {"available": True, "busy_process_threshold_percent": 25,
        "busy_processes": [], "load_average": [0, 0, 0]}


def handshake(options):
    return ["id name Sekirei synthetic"] + [
        "option name " + name + (" type combo default Auto var Auto var Speculative var LazySMP"
                                if name == "SearchMode" else " type string default fixture")
        for name in options
    ] + ["usiok"]


def speculative_metrics(workers=4):
    return dict(version=1, configured_workers=workers, background_cap=workers - 1,
                candidates=8, admitted=6, started=6, completed=2, aborted=4, unwound=0,
                unstarted=0, cap_rejected=2, peak_in_flight=3, outstanding_after_join=0,
                background_nodes=400, elapsed_ns=301_200_000, scope_join_wait_ns=100_000,
                deadline_overrun_ns=1_200_000, cancellation_latency_samples=3,
                cancellation_latency_ns_sum=400_000, cancellation_latency_ns_max=200_000,
                aborted_without_observed_cancel=1, cancellation_scope="explicit_request_to_capture_release")


def tt_use_metrics():
    return dict(version=1, available=1, score_hits_from_speculation=9, move_hits_from_speculation=6,
                unattributed_score_hits=3, main_cutoffs_from_speculation=3,
                quiescence_cutoffs_from_speculation=2, slack_cutoffs_from_speculation=1,
                probcut_cutoffs_from_speculation=1, nonroot_move_first_started=4,
                nonroot_move_first_completed=3, nonroot_move_first_beta_cuts=1,
                source_scope="current_invocation_speculation", consumer_scope="foreground_tree",
                ordering_scope="nonroot_first_child", benefit_scope="observed_kernel_use")


def record(tag, fields):
    return "info string " + tag + " " + " ".join(f"{key} {value}" for key, value in fields.items())


def lines(side="B", workers=4, lane="timing"):
    result = ["info depth 5 score cp 23 nodes 1000 time 301 pv 7g7f 3c3d"]
    if side == "A":
        stats = ",".join(f"w{worker}{'*' if worker == 0 else ''}:d5:n{1000 // workers}:s23:t300:x1:abudget:p0:h0:r0"
                         for worker in range(workers))
        result.append(f"info string lazy_smp selected w0 node_share_permille 250 move_agreement 4/{workers} "
                      "discarded_root_nodes 750 divergent_discarded_nodes 0 stop_lag_ms 1 "
                      + stats)
        if lane == "counters":
            result += [record("lazy_smp_tt", dict(zip(driver.LAZY_TT_FIELDS, (100, 40, 70, 50, 30, 5, 10, 20)))),
                       record("lazy_smp_overlap", dict(zip(driver.LAZY_OVERLAP_FIELDS, (256, 100, 80, 10, 10, 5, 0, 20, 12, 5, 4, 3, 0))))]
    elif lane == "counters":
        result.append(record("speculative_tt_use", tt_use_metrics()))
        result.append(record("speculative_diagnostics", speculative_metrics(workers)))
    return result + ["bestmove 7g7f ponder 3c3d"]


class Protocol:
    instances = []

    def __init__(self, binary, workers, directory, timeout):
        self.commands = []
        self.closed = False
        self.workers = workers
        self.options = driver.configuration("B", workers, 64)["options"]
        (directory / "stderr.txt").write_text("")
        self.instances.append(self)

    def send(self, command):
        self.commands.append(command)

    def wait_for(self, token):
        if token == "usiok":
            return handshake(self.options)
        if token == "readyok":
            return ["readyok"]
        side = "A" if "setoption name SearchMode value LazySMP" in self.commands else "B"
        lane = "counters" if any(command in self.commands for command in (
            "setoption name SpeculativeDiagnostics value true", "setoption name LazyDiagnostics value true")) else "timing"
        return lines(side, self.workers, lane)

    def close(self):
        self.closed = True
        return {"exitcode": 0, "killed": False, "reaped": True, "reader_finished": True}


class Contracts(unittest.TestCase):
    def test_counter_options_and_runtime_records_are_explicit_and_mode_specific(self):
        for workers in (4, 8):
            for side in "AB":
                config = driver.configuration(side, workers, 64, "counters")
                self.assertEqual(config["options"]["SpeculativeDiagnostics"], str(side == "B").lower())
                self.assertEqual(config["options"]["LazyDiagnostics"], str(side == "A").lower())
                row = driver.parse_result(lines(side, workers, "counters"), 310, config, 300)
                self.assertFalse(row["performance_comparison_eligible"])
                self.assertIsNone(row["wall_nps"])
                self.assertEqual(row["useful_tt_cutoff_attribution"], "not_measured")
                self.assertEqual(row["speculative_runtime_pool_count_observed"], side == "B")
                self.assertEqual(row["tt_publication_use_observed"], side == "B")
                if side == "B":
                    self.assertEqual(row["speculative_diagnostics"]["background_nodes"], 400)
                    self.assertEqual(row["speculative_diagnostics"]["aborted_without_observed_cancel"], 1)
                else:
                    self.assertEqual(row["lazy_smp_overlap"]["tt_foreign_hits"], 4)

    def test_diagnostic_lane_mixing_and_missing_reports_fail_closed(self):
        for side in "AB":
            with self.assertRaisesRegex(ValueError, "timing lane"):
                driver.parse_result(lines(side, lane="counters"), 310, driver.configuration(side, 4, 64), 300)
            with self.assertRaisesRegex(ValueError, "counter lane"):
                driver.parse_result(lines(side), 310, driver.configuration(side, 4, 64, "counters"), 300)
        mixed = lines("B", lane="counters")[:-1] + [record("lazy_smp_tt", dict.fromkeys(driver.LAZY_TT_FIELDS, 0))] + ["bestmove 7g7f"]
        with self.assertRaisesRegex(ValueError, "counter lane"):
            driver.parse_result(mixed, 310, driver.configuration("B", 4, 64, "counters"), 300)

    def test_speculative_parser_rejects_unsound_task_time_and_version_metrics(self):
        cases = {"version": 2, "configured_workers": 8, "background_cap": 7,
                 "candidates": 9, "admitted": 7, "completed": 3, "started": 5,
                 "unwound": 1, "outstanding_after_join": 1, "peak_in_flight": 4,
                 "background_nodes": 1001, "cancellation_latency_samples": 4,
                 "cancellation_latency_ns_sum": 1, "cancellation_latency_ns_max": 500_000,
                 "scope_join_wait_ns": 302_000_000, "elapsed_ns": 302_000_000,
                 "deadline_overrun_ns": "none", "aborted_without_observed_cancel": -1,
                 "cancellation_scope": "guessed_external_stop"}
        config = driver.configuration("B", 4, 64, "counters")
        for key, value in cases.items():
            with self.subTest(key=key):
                fields = speculative_metrics()
                fields[key] = value
                damaged = [line for line in lines(lane="counters")[:-1] if not line.startswith("info string speculative_diagnostics ")] + [record("speculative_diagnostics", fields), "bestmove 7g7f"]
                with self.assertRaises(ValueError):
                    driver.parse_result(damaged, 310, config, 300)
        original = lines(lane="counters")
        diagnostic = original[-2]
        for bad in (diagnostic + " admitted 6", diagnostic.replace(" version 1", " version"),
                    diagnostic.replace(" started 6", " admitted 6")):
            with self.assertRaises(ValueError):
                driver.parse_result(original[:-2] + [bad, original[-1]], 310, config, 300)
        with self.assertRaises(ValueError):
            driver.parse_result(original[:-1] + [diagnostic, original[-1]], 310, config, 300)

    def test_counter_protocol_sends_option_and_parses_real_record_shape(self):
        with tempfile.TemporaryDirectory() as tmp, patch.object(driver, "TranscriptEngine", Protocol):
            row = driver.run_slot(Path(tmp) / "engine", "startpos", driver.configuration("B", 8, 64, "counters"), 300, 5, Path(tmp))
        protocol = Protocol.instances[-1]
        self.assertIn("setoption name SpeculativeDiagnostics value true", protocol.commands)
        self.assertIn("setoption name LazyDiagnostics value false", protocol.commands)
        self.assertIn("setoption name SpecTopN value 7", protocol.commands)
        self.assertTrue(row["speculative_runtime_pool_count_observed"])
        self.assertEqual(row["speculative_tt_use"]["probcut_cutoffs_from_speculation"], 1)

    def test_legacy_task_only_counter_logs_remain_unmeasured_not_zero_attributed(self):
        legacy_lines = [line for line in lines(lane="counters") if not line.startswith("info string speculative_tt_use ")]
        current = driver.configuration("B", 4, 64, "counters")
        with self.assertRaisesRegex(ValueError, "requires speculative_tt_use"):
            driver.parse_result(legacy_lines, 310, current, 300)
        legacy = dict(current)
        del legacy["speculative_tt_use_required"]
        row = driver.parse_result(legacy_lines, 310, legacy, 300)
        self.assertIsNone(row["speculative_tt_use"])
        self.assertFalse(row["tt_publication_use_observed"])
        self.assertEqual(row["useful_tt_cutoff_attribution"], "not_measured")
        self.assertEqual(row["speculative_diagnostics"], speculative_metrics())
        expected = list(driver.schedule([4], 1, 1))
        rows = []
        for repeat, workers, position, slot, side in expected:
            historical = dict(row) if side == "B" else driver.parse_result(lines("A", lane="counters"), 310, driver.configuration("A", 4, 64, "counters"), 300)
            historical.update(repeat=repeat, workers=workers, position=position, abba_slot=slot, side=side, root_sha256="legacy-root")
            rows.append(historical)
        summary = driver.summarize(rows, expected, "counters")["by_workers"]["4"]
        self.assertIsNone(summary["speculative_tt_use_totals"])
        self.assertEqual(summary["speculative_tt_use_coverage"], {"observed_slots": 0, "total_slots": 2})

    def test_tt_use_parser_requires_exact_version_scope_and_event_accounting(self):
        config = driver.configuration("B", 4, 64, "counters")
        original = lines(lane="counters")
        for key, bad in {"version": 2, "available": 0, "score_hits_from_speculation": 6,
                         "main_cutoffs_from_speculation": 10, "unattributed_score_hits": -1,
                         "nonroot_move_first_started": 7, "nonroot_move_first_completed": 5,
                         "nonroot_move_first_beta_cuts": 4, "benefit_scope": "saved_nodes",
                         "ordering_scope": "all_root_moves", "consumer_scope": "any_worker"}.items():
            with self.subTest(key=key):
                fields = tt_use_metrics()
                fields[key] = bad
                damaged = [record("speculative_tt_use", fields) if line.startswith("info string speculative_tt_use ") else line for line in original]
                with self.assertRaises(ValueError):
                    driver.parse_result(damaged, 310, config, 300)
        tt_line = next(line for line in original if line.startswith("info string speculative_tt_use "))
        for extra in (tt_line, tt_line + " available 1", tt_line.replace(" version 1", " version")):
            with self.assertRaises(ValueError):
                driver.parse_result(original[:-1] + [extra, original[-1]], 310, config, 300)
        with self.assertRaisesRegex(ValueError, "timing lane"):
            driver.parse_result(lines()[:-1] + [tt_line, "bestmove 7g7f"], 310, driver.configuration("B", 4, 64), 300)

    def test_lazy_counter_parser_rejects_duplicate_and_inconsistent_observations(self):
        config = driver.configuration("A", 4, 64, "counters")
        original = lines("A", lane="counters")
        for before, after in (("hits 40", "hits 101"), ("committed 50", "committed 71"),
                              ("sample_rate 256", "sample_rate 1"), ("tt_foreign_hits 4", "tt_foreign_hits 5")):
            with self.subTest(before=before), self.assertRaises(ValueError):
                driver.parse_result([line.replace(before, after) for line in original], 310, config, 300)
        with self.assertRaises(ValueError):
            driver.parse_result(original[:-1] + [original[-2], original[-1]], 310, config, 300)

    def test_summary_rejects_mixed_instrumentation_lanes(self):
        expected = list(driver.schedule([4], 1, 1))
        rows = []
        for repeat, workers, position, slot, side in expected:
            row = driver.parse_result(lines(side, lane="counters"), 310, driver.configuration(side, workers, 64, "counters"), 300)
            row.update(repeat=repeat, workers=workers, position=position, abba_slot=slot, side=side, root_sha256="root")
            rows.append(row)
        rows[0]["measurement_lane"] = "timing"
        with self.assertRaisesRegex(ValueError, "lanes must not be mixed"):
            driver.summarize(rows, expected, "counters")

    def test_complete_counter_manifest_keeps_timings_unqualified_and_no_paired_performance_delta(self):
        def fake_slot(binary, root, config, movetime, timeout, directory):
            return driver.parse_result(lines(config["side"], config["configured_search_workers"], "counters"), 310, config, movetime)
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with patch.object(driver, "run_slot", side_effect=fake_slot), patch.object(driver, "host_activity", return_value=IDLE):
                self.assertEqual(driver.main(self.fixture(root) + ["--lane", "counters", "--threads", "4,8"]), 0)
            manifest = json.loads((root / "out/manifest.json").read_text())
            summary = json.loads((root / "out/summary.json").read_text())
            self.assertEqual(manifest["status"], "complete")
            self.assertTrue(manifest["counter_lane"])
            self.assertFalse(manifest["timing_qualified"])
            self.assertIn("not comparable performance", manifest["observer_contract"])
            self.assertFalse(summary["performance_comparison_eligible"])
            self.assertNotIn("median_paired_delta", json.dumps(summary))
            self.assertEqual(summary["by_workers"]["4"]["speculative_totals"]["admitted"], 24)
            self.assertEqual(summary["by_workers"]["8"]["lazy_overlap_totals"]["tt_foreign_hits"], 16)
            self.assertEqual(summary["by_workers"]["4"]["speculative_tt_use_totals"]["score_hits_from_speculation"], 36)
            self.assertEqual(summary["by_workers"]["4"]["speculative_tt_use_totals"]["unattributed_score_hits"], 12)
            self.assertEqual(summary["by_workers"]["4"]["speculative_tt_use_coverage"], {"observed_slots": 4, "total_slots": 4})
            self.assertEqual({row["measurement_lane"] for row in map(json.loads, (root / "out/rows.jsonl").read_text().splitlines())}, {"counters"})

    def test_pv_validator_replays_root_moves_before_the_reported_pv(self):
        row = {"bestmove": "3c3d", "pv": ["3c3d"]}
        start = pv_validator.requests_for_row("startpos moves 7g7f", row)
        self.assertEqual(start, [(pv_validator.apply(pv_validator.START, "7g7f"), "3c3d", 0)])
        full = pv_validator.requests_for_row("sfen " + pv_validator.START + " moves 7g7f", row)
        self.assertEqual(full, start)
        with self.assertRaisesRegex(ValueError, "invalid root move list"):
            pv_validator.requests_for_row("startpos moves", row)

    def test_complete_speculative_run_can_be_checked_by_the_legal_pv_tool(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            run = root / "run"
            run.mkdir()
            corpus = run / "positions.input"
            corpus.write_text("startpos moves 7g7f\n")
            engine = run / "engine"
            engine.write_bytes(b"frozen synthetic engine")
            (run / "manifest.json").write_text(json.dumps({
                "schema": driver.SCHEMA + ".manifest", "status": "complete",
                "positions_sha256": driver.sha256(corpus),
                "engine": {"path": str(engine), "sha256": driver.sha256(engine)},
                "expected_searches": 1, "completed_searches": 1,
            }))
            (run / "rows.jsonl").write_text(json.dumps({
                "position": 0, "sequence": 0, "bestmove": "3c3d", "pv": ["3c3d"],
            }) + "\n")
            legal = root / "legal-moves"
            legal.write_bytes(b"frozen legal move checker fixture")
            output = root / "validation.json"
            argv = ["validate_paired_pv.py", "--legal-moves", str(legal), "--output", str(output), str(run)]
            with patch("sys.argv", argv), patch.object(pv_validator, "probe", return_value=["3c3d"]):
                self.assertFalse(pv_validator.main())
            report = json.loads(output.read_text())
            self.assertEqual(report["pv_moves"], 1)
            self.assertEqual(report["violations"], [])
            self.assertEqual(report["runs"][0]["engine_sha256"], {"A": driver.sha256(engine), "B": driver.sha256(engine)})

    def fixture(self, root):
        binary, positions = root / "binary", root / "roots.sfen"
        binary.write_bytes(b"frozen synthetic binary; must never execute")
        positions.write_text("startpos\nstartpos moves 7g7f\n")
        return ["--binary", str(binary), "--positions", str(positions),
                "--output", str(root / "out"), "--repeats", "1",
                "--resource-root", str(root), "--minimum-free-mib", "1"]

    def test_worker_counts_have_real_speculative_top_n_and_one_shared_budget(self):
        for workers in (4, 8):
            baseline = driver.configuration("A", workers, 64)
            candidate = driver.configuration("B", workers, 64)
            self.assertEqual(baseline["options"]["SearchMode"], "LazySMP")
            self.assertEqual(candidate["options"]["SearchMode"], "Speculative")
            self.assertEqual(candidate["options"]["SpecTopN"], str(workers - 1))
            self.assertEqual(candidate["dedicated_speculative_pool_threads"], workers)
            for name in set(baseline["options"]) - {"SearchMode", "SpecTopN"}:
                self.assertEqual(baseline["options"][name], candidate["options"][name])
            self.assertNotEqual(baseline["node_accounting"], candidate["node_accounting"])

    def test_prepare_only_freezes_reviewable_plan_without_process_or_admission(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            args = self.fixture(root) + ["--threads", "4,8", "--prepare-only"]
            with patch.object(driver, "run_slot") as run, patch.object(driver, "resource_snapshot") as admission:
                self.assertEqual(driver.main(args), 0)
            run.assert_not_called()
            admission.assert_not_called()
            manifest = json.loads((root / "out/manifest.json").read_text())
            self.assertEqual(manifest["status"], "prepared")
            self.assertEqual(manifest["expected_searches"], 16)
            self.assertFalse(manifest["strength_gate_opened"])
            self.assertFalse(manifest["timing_qualified"])
            self.assertEqual(manifest["budget"]["go_movetime_ms"], 300)
            self.assertIsNone(manifest["budget"]["node_limit"])
            self.assertEqual(len(manifest["configuration_sha256"]), 64)
            self.assertEqual(set(manifest["measurement_source_sha256"]), set(driver.SOURCES))
            self.assertEqual(driver.sha256(root / "out/engine"), manifest["engine"]["sha256"])
            self.assertFalse((root / "out/rows.jsonl").exists())

    def test_protocol_uses_ready_barriers_and_one_fresh_root(self):
        with tempfile.TemporaryDirectory() as tmp, patch.object(driver, "TranscriptEngine", Protocol):
            root = Path(tmp)
            result = driver.run_slot(root / "engine", "startpos moves 7g7f", driver.configuration("B", 4, 64), 300, 5, root)
        protocol = Protocol.instances[-1]
        self.assertTrue(protocol.closed)
        self.assertIn("setoption name SpecTopN value 3", protocol.commands)
        self.assertIn("setoption name Threads value 4", protocol.commands)
        self.assertNotIn("setoption name EvalFile", "\n".join(protocol.commands))
        self.assertEqual(protocol.commands[-5:], ["isready", "usinewgame", "isready", "position startpos moves 7g7f", "go movetime 300"])
        self.assertEqual(result["bestmove"], "7g7f")
        self.assertEqual(result["engine_overrun_ms"], 1)
        self.assertFalse(result["speculative_runtime_pool_count_observed"])

    def test_protocol_failure_still_records_shutdown(self):
        class FailedProtocol(Protocol):
            def wait_for(self, token):
                if token == "bestmove":
                    raise TimeoutError("synthetic deadline")
                return super().wait_for(token)
        with tempfile.TemporaryDirectory() as tmp, patch.object(driver, "TranscriptEngine", FailedProtocol):
            root = Path(tmp)
            with self.assertRaisesRegex(TimeoutError, "synthetic"):
                driver.run_slot(root / "engine", "startpos", driver.configuration("B", 4, 64), 300, 5, root)
            self.assertTrue(json.loads((root / "shutdown.json").read_text())["reaped"])
            self.assertTrue(Protocol.instances[-1].closed)

    def test_actual_lazy_worker_count_and_mode_are_required(self):
        self.assertEqual(driver.parse_result(lines("A", 4), 310, driver.configuration("A", 4, 64), 300)["lazy_smp"]["workers"], 4)
        with self.assertRaisesRegex(ValueError, "worker report"):
            driver.parse_result(lines("A", 4), 310, driver.configuration("A", 8, 64), 300)
        with self.assertRaisesRegex(ValueError, "unexpectedly"):
            driver.parse_result(lines("A", 4), 310, driver.configuration("B", 4, 64), 300)
        with self.assertRaisesRegex(ValueError, "worker report"):
            driver.parse_result(lines(), 310, driver.configuration("A", 4, 64), 300)
        damaged = [line.replace("w3:d5", "w2:d5") for line in lines("A", 4)]
        with self.assertRaisesRegex(ValueError, "worker records"):
            driver.parse_result(damaged, 310, driver.configuration("A", 4, 64), 300)

    def test_unadvertised_options_and_evaluator_loading_fail_closed(self):
        requested = driver.configuration("B", 4, 64)["options"]
        without_book = {key: value for key, value in requested.items() if key != "UseBook"}
        self.assertNotIn("UseBook", driver.admitted_options(handshake(without_book), requested))
        without_top_n = {key: value for key, value in requested.items() if key != "SpecTopN"}
        with self.assertRaisesRegex(ValueError, "SpecTopN"):
            driver.admitted_options(handshake(without_top_n), requested)
        wrong_mode = [line.replace("var Speculative", "var SpeculativeOther") for line in handshake(requested)]
        with self.assertRaisesRegex(ValueError, "search mode"):
            driver.admitted_options(wrong_mode, requested)
        for message in ("info string NNUE weights loaded from private.bin", "info string cannot set Threads", "info string invalid SpecTopN"):
            with self.assertRaises(ValueError):
                driver.check_messages([message])

    def test_missing_external_root_stops_before_engine_start(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            args = self.fixture(root)
            args[args.index("--resource-root") + 1] = str(root / "absent-external-root")
            with patch.object(driver, "run_slot") as run, patch.object(driver, "host_activity", return_value=IDLE):
                self.assertEqual(driver.main(args), 2)
            run.assert_not_called()
            manifest = json.loads((root / "out/manifest.json").read_text())
            self.assertEqual(manifest["status"], "blocked_admission")
            self.assertEqual(manifest["completed_searches"], 0)
            self.assertFalse(manifest["last_admission"]["resources"]["paths"]["resource_root"]["admitted"])

    def test_complete_abba_uses_unique_process_slots_and_independent_worker_reports(self):
        seen = []
        def fake_slot(binary, root, config, movetime, timeout, directory):
            seen.append((str(binary), root, config, movetime, directory))
            return {"depth": 10 if config["side"] == "A" else 12, "nodes": 100,
                    "elapsed_ms": 301, "response_overrun_ms": 1}
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            args = self.fixture(root) + ["--threads", "4,8"]
            with patch.object(driver, "run_slot", side_effect=fake_slot), patch.object(driver, "host_activity", return_value=IDLE):
                self.assertEqual(driver.main(args), 0)
            manifest = json.loads((root / "out/manifest.json").read_text())
            summary = json.loads((root / "out/summary.json").read_text())
            self.assertEqual(manifest["completed_searches"], 16)
            self.assertEqual([record[2]["side"] for record in seen], list("ABBA" * 4))
            self.assertEqual(len({record[4] for record in seen}), 16)
            self.assertEqual(len({record[0] for record in seen}), 1)
            self.assertEqual({record[3] for record in seen}, {300})
            self.assertEqual(set(summary["by_workers"]), {"4", "8"})
            self.assertEqual(summary["by_workers"]["4"]["median_paired_delta"]["depth"], 2)
            self.assertFalse(summary["strength_gain_proven"])
            with self.assertRaises(FileExistsError):
                driver.main(args)

    def test_interrupted_abba_is_preserved_and_never_summarized_or_retried(self):
        calls = []
        def fake_slot(*args):
            calls.append(args)
            if len(calls) == 3:
                raise RuntimeError("injected engine failure")
            return {"depth": 10, "nodes": 100, "elapsed_ms": 301, "response_overrun_ms": 1}
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with patch.object(driver, "run_slot", side_effect=fake_slot), patch.object(driver, "host_activity", return_value=IDLE):
                with self.assertRaisesRegex(RuntimeError, "injected"):
                    driver.main(self.fixture(root))
            manifest = json.loads((root / "out/manifest.json").read_text())
            self.assertEqual(manifest["status"], "incomplete")
            self.assertEqual(manifest["completed_searches"], 2)
            self.assertEqual(len((root / "out/rows.jsonl").read_text().splitlines()), 2)
            self.assertFalse((root / "out/summary.json").exists())
            self.assertEqual(len(calls), 3)

    def test_summary_rejects_missing_or_changed_root_slots(self):
        expected = list(driver.schedule([4], 1, 1))
        rows = [{"repeat": repeat, "workers": count, "position": position, "abba_slot": slot,
                 "side": side, "root_sha256": "root", "depth": 10, "nodes": 10,
                 "elapsed_ms": 300, "response_overrun_ms": 0}
                for repeat, count, position, slot, side in expected]
        with self.assertRaisesRegex(ValueError, "incomplete"):
            driver.summarize(rows[:-1], expected)
        rows[1]["root_sha256"] = "different"
        with self.assertRaisesRegex(ValueError, "root mismatch"):
            driver.summarize(rows, expected)

    def test_last_slot_input_drift_is_incomplete_even_when_every_search_returned(self):
        calls = []
        def fake_slot(binary, root, config, movetime, timeout, directory):
            calls.append(directory)
            if len(calls) == 8:
                (directory.parent / "positions.input").write_text("changed after final admission")
            return {"depth": 10, "nodes": 100, "elapsed_ms": 301, "response_overrun_ms": 1}
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with patch.object(driver, "run_slot", side_effect=fake_slot), patch.object(driver, "host_activity", return_value=IDLE):
                with self.assertRaisesRegex(ValueError, "frozen inputs"):
                    driver.main(self.fixture(root))
            manifest = json.loads((root / "out/manifest.json").read_text())
            self.assertEqual(manifest["status"], "incomplete")
            self.assertEqual(manifest["completed_searches"], 8)
            self.assertFalse((root / "out/summary.json").exists())

    def test_corpus_rejects_duplicate_or_injected_commands(self):
        self.assertEqual(driver.positions_from_text("# comment\nstartpos moves 7g7f 3c3d\n"), ["startpos moves 7g7f 3c3d"])
        for text in ("startpos\nstartpos", "startpos\nquit", "startpos moves quit", "startpos; quit"):
            with self.assertRaises(ValueError):
                driver.positions_from_text(text)


if __name__ == "__main__":
    unittest.main()
