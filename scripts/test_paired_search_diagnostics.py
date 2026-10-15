import unittest
from unittest.mock import patch
from types import SimpleNamespace

from paired_search_diagnostics import schedule, side_changes, summarize, host_activity, validate_counter_workers, work_metrics, main as paired_main
from generate_search_diagnostic import START, apply, decode, encode
from validate_paired_pv import requests_for_row
from nps_threads import ROOT_HINT_TRANSFER_KEYS, parse_lazy_smp_root_hint, parse_retirement, parse_search_result, parse_nmp, parse_cooperative_root, parse_aspiration


class PairedDiagnosticsTests(unittest.TestCase):
    def test_host_activity_records_external_load_without_mutation(self):
        with patch("paired_search_diagnostics.subprocess.run", return_value=SimpleNamespace(stdout="PID %CPU COMM\n23 99.0 rustc\n24 0.1 idle\n")) as run:
            result = host_activity()
        self.assertEqual(result["busy_processes"], [{"pid": 23, "cpu_percent": 99.0, "command": "rustc"}])
        self.assertEqual(run.call_args.args[0], ["ps", "-Ao", "pid,pcpu,comm"])
        with patch("paired_search_diagnostics.subprocess.run", side_effect=OSError("denied")):
            self.assertFalse(host_activity()["available"])

    def test_schedule_interleaves_each_position_in_abba_order(self):
        rows = list(schedule([1, 4], 2, 2))
        self.assertEqual(len(rows), 32)
        self.assertEqual([row[4] for row in rows[:8]], list("ABBAABBA"))
        self.assertEqual([row[2] for row in rows[:8]], [0] * 4 + [1] * 4)
        self.assertEqual([row[1] for row in rows[:8]], [1] * 8)

    def test_busy_preflight_writes_complete_identity_and_driver_provenance(self):
        import json
        from pathlib import Path
        from tempfile import TemporaryDirectory
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            engine = root / "engine"
            engine.write_bytes(b"frozen engine fixture")
            positions = root / "positions.sfen"
            positions.write_text("startpos\n")
            output = root / "out"
            args = ["paired_search_diagnostics.py", "--baseline", str(engine),
                    "--candidate", str(engine), "--positions", str(positions),
                    "--output", str(output), "--threads", "1", "--count", "1"]
            busy = {"available": True, "busy_processes": [{"pid": 123, "cpu_percent": 100}], "load_average": [1, 1, 1]}
            with patch("sys.argv", args), patch("paired_search_diagnostics.host_activity", return_value=busy), patch("builtins.print"):
                self.assertEqual(paired_main(), 2)
            manifest = json.loads((output / "manifest.json").read_text())
            self.assertEqual(manifest["status"], "blocked_host_busy")
            self.assertEqual(manifest["identity_policy"], "require_1T_fixed_depth")
            self.assertEqual(len(manifest["configuration_sha256"]), 64)
            self.assertEqual(set(manifest["measurement_source_sha256"]), {"paired_search_diagnostics.py", "nps_threads.py"})
            self.assertFalse((output / "rows.jsonl").exists())

    def test_one_worker_diagnostics_have_no_retirement_channel(self):
        row = {"retirement": [], "nmp": [{"worker": 0}], "aspiration": [{"worker": 0}]}
        validate_counter_workers(row, 1)
        with self.assertRaisesRegex(ValueError, "retirement"):
            validate_counter_workers(row, 2)
        with self.assertRaisesRegex(ValueError, "aspiration"):
            validate_counter_workers({**row, "aspiration": []}, 1)
        two = {key: [{"worker": 0}, {"worker": 1}] for key in row}
        validate_counter_workers(two, 2)

    def test_wait_activity_metrics_do_not_require_regular_lazy_counters(self):
        baseline = dict(depth=8, nodes=100, wall_nps=1000, elapsed_ms=100)
        self.assertEqual(work_metrics(baseline), baseline)
        waiting = dict(enabled=1, waiting_confirmations=2, waiting_confirmation_nodes=30,
                       waiting_alpha_raises=1, notification_wait_us=500)
        result = work_metrics({**baseline, "cooperative_root_wait": waiting})
        self.assertEqual(result["coop_waiting_confirmations"], 2)
        self.assertEqual(result["coop_waiting_confirmation_nodes"], 30)
        self.assertEqual(result["coop_waiting_alpha_raises"], 1)
        self.assertEqual(result["coop_notification_wait_us"], 500)
        self.assertNotIn("coop_enabled", result)
        self.assertNotIn("worker0_depth", result)

    def test_latency_activity_metrics_are_separate_from_wait_drain(self):
        baseline = dict(depth=10, nodes=100, wall_nps=1000, elapsed_ms=100)
        latency = dict(enabled=1, publications=4, confirmations=2,
                       confirmation_main_nodes_sum=120, unconfirmed_publications=2)
        result = work_metrics({**baseline, "cooperative_root_latency": latency})
        self.assertEqual(result["coop_latency_publications"], 4)
        self.assertEqual(result["coop_latency_confirmations"], 2)
        self.assertEqual(result["coop_latency_confirmation_main_nodes_sum"], 120)
        self.assertNotIn("coop_latency_enabled", result)
        self.assertNotIn("coop_waiting_confirmations", result)

    def test_latency_instrumentation_disqualifies_timing_without_regular_lazy_counters(self):
        import json
        from pathlib import Path
        from tempfile import TemporaryDirectory
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            engine = root / "engine"
            engine.write_bytes(b"frozen engine fixture")
            positions = root / "positions.sfen"
            positions.write_text("startpos\n")
            output = root / "out"
            args = ["paired_search_diagnostics.py", "--baseline", str(engine),
                    "--candidate", str(engine), "--positions", str(positions),
                    "--output", str(output), "--threads", "2", "--count", "1", "--repeats", "1", "--depth", "10",
                    "--a-option", "T_COOP_LATENCY=0", "--b-option", "T_COOP_LATENCY=1"]
            idle = {"available": True, "busy_processes": [], "load_average": [0, 0, 0]}
            observed = []
            def fake_measure(lane, threads, positions):
                observed.append(lane)
                return [dict(threads=threads, depth=10, nodes=100, wall_nps=1000, elapsed_ms=100,
                             bestmove="7g7f", score=["cp", "0"], pv=["7g7f"])]
            with patch("sys.argv", args), patch("paired_search_diagnostics.host_activity", return_value=idle), patch("paired_search_diagnostics.measure", side_effect=fake_measure), patch("builtins.print"):
                self.assertEqual(paired_main(), 0)
            manifest = json.loads((output / "manifest.json").read_text())
            self.assertEqual(manifest["status"], "complete")
            self.assertFalse(manifest["timing_qualified"])
            self.assertFalse(manifest["counter_lane"])
            self.assertEqual(manifest["budget"], {"depth": 10, "movetime_ms": None})
            self.assertNotIn("measurement_phase", manifest)
            self.assertEqual(manifest["cooperative_latency_instrumentation_requested"], {"A": False, "B": True})
            self.assertEqual([lane.provenance["cooperative_latency_instrumentation_requested"] for lane in observed], [False, True, True, False])
            self.assertTrue(all(not lane.require_lazy_diagnostics for lane in observed))
            self.assertEqual([lane.provenance["counter_overhead_included"] for lane in observed], [False, True, True, False])
            busy_output = root / "busy-out"
            busy_args = [value for value in args]
            busy_args[busy_args.index("--output") + 1] = str(busy_output)
            depth_index = busy_args.index("--depth")
            del busy_args[depth_index:depth_index + 2]
            busy = {**idle, "busy_processes": [{"pid": 123, "cpu_percent": 100}]}
            with patch("sys.argv", busy_args), patch("paired_search_diagnostics.host_activity", return_value=busy), patch("paired_search_diagnostics.measure") as measurement, patch("builtins.print"):
                self.assertEqual(paired_main(), 2)
                measurement.assert_not_called()
            self.assertEqual(json.loads((busy_output / "manifest.json").read_text())["status"], "blocked_host_busy")

    def test_priority_outcome_metrics_use_actual_activation_without_latency(self):
        baseline = dict(depth=8, nodes=100, wall_nps=1000, elapsed_ms=100, cooperative_root={})
        self.assertEqual(work_metrics(baseline)["coop_priority_enabled"], 0)
        observed = work_metrics({**baseline, "cooperative_root_policy": {"priority": 1},
                                 "cooperative_root_outcomes": {"enabled": 1, "priority_choices": 3,
                                                                "reordered_confirmation_nodes": 20}})
        self.assertEqual(observed["coop_priority_enabled"], 1)
        self.assertEqual(observed["coop_outcomes_priority_choices"], 3)
        self.assertEqual(observed["coop_outcomes_reordered_confirmation_nodes"], 20)
        self.assertNotIn("coop_outcomes_enabled", observed)
        self.assertNotIn("coop_latency_publications", observed)

    def test_priority_without_observer_retains_counter_free_timing_qualification(self):
        import json
        from pathlib import Path
        from tempfile import TemporaryDirectory
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            engine = root / "engine"
            engine.write_bytes(b"frozen engine fixture")
            positions = root / "positions.sfen"
            positions.write_text("startpos\n")
            output = root / "out"
            args = ["paired_search_diagnostics.py", "--baseline", str(engine),
                    "--candidate", str(engine), "--positions", str(positions),
                    "--output", str(output), "--threads", "2", "--count", "1", "--repeats", "1",
                    "--option", "T_COOP_LATENCY=0", "--a-option", "T_COOP_PRIORITY=0", "--b-option", "T_COOP_PRIORITY=1"]
            idle = {"available": True, "busy_processes": [], "load_average": [0, 0, 0]}
            observed = []
            def fake_measure(lane, threads, positions):
                observed.append(lane)
                enabled = "T_COOP_PRIORITY=1" in lane.option
                return [dict(threads=threads, depth=8, nodes=100, wall_nps=1000, elapsed_ms=100,
                             bestmove="7g7f", score=["cp", "0"], pv=["7g7f"], cooperative_root={},
                             cooperative_root_policy={"priority": 1} if enabled else None)]
            with patch("sys.argv", args), patch("paired_search_diagnostics.host_activity", return_value=idle), patch("paired_search_diagnostics.measure", side_effect=fake_measure), patch("builtins.print"):
                self.assertEqual(paired_main(), 0)
            manifest = json.loads((output / "manifest.json").read_text())
            self.assertTrue(manifest["timing_qualified"])
            self.assertFalse(manifest["counter_lane"])
            self.assertEqual(manifest["cooperative_priority_requested"], {"A": False, "B": True})
            self.assertEqual([lane.provenance["cooperative_priority_requested"] for lane in observed], [False, True, True, False])
            self.assertTrue(all(not lane.provenance["counter_overhead_included"] for lane in observed))
            summary = json.loads((output / "summary.json").read_text())
            self.assertEqual(summary["by_threads"]["2"]["median_paired_delta"]["coop_priority_enabled"], 1)

    def test_helper_lmr_metrics_record_actual_activation_and_optional_counts(self):
        baseline = dict(depth=8, nodes=100, wall_nps=1000, elapsed_ms=100, cooperative_root={})
        self.assertEqual(work_metrics(baseline)["coop_helper_lmr_enabled"], 0)
        observed = work_metrics({**baseline, "cooperative_root_helper_lmr_policy": {"enabled": 1},
                                 "cooperative_root_helper_lmr": {"reduced_probes": 4, "completed_reduced_fail_lows": 3}})
        self.assertEqual(observed["coop_helper_lmr_enabled"], 1)
        self.assertEqual(observed["coop_helper_lmr_reduced_probes"], 4)
        self.assertEqual(observed["coop_helper_lmr_completed_reduced_fail_lows"], 3)
        self.assertNotIn("coop_latency_publications", observed)

    def test_helper_lmr_without_observer_keeps_counter_free_timing_lane(self):
        import json
        from pathlib import Path
        from tempfile import TemporaryDirectory
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            engine = root / "engine"
            engine.write_bytes(b"frozen engine fixture")
            positions = root / "positions.sfen"
            positions.write_text("startpos\n")
            output = root / "out"
            args = ["paired_search_diagnostics.py", "--baseline", str(engine),
                    "--candidate", str(engine), "--positions", str(positions),
                    "--output", str(output), "--threads", "2", "--count", "1", "--repeats", "1",
                    "--option", "T_COOP_LATENCY=0", "--a-option", "T_COOP_HELPER_LMR=0", "--b-option", "T_COOP_HELPER_LMR=1"]
            idle = {"available": True, "busy_processes": [], "load_average": [0, 0, 0]}
            observed = []
            def fake_measure(lane, threads, positions):
                observed.append(lane)
                enabled = "T_COOP_HELPER_LMR=1" in lane.option
                return [dict(threads=threads, depth=8, nodes=100, wall_nps=1000, elapsed_ms=100,
                             bestmove="7g7f", score=["cp", "0"], pv=["7g7f"], cooperative_root={},
                             cooperative_root_helper_lmr_policy={"enabled": 1} if enabled else None)]
            with patch("sys.argv", args), patch("paired_search_diagnostics.host_activity", return_value=idle), patch("paired_search_diagnostics.measure", side_effect=fake_measure), patch("builtins.print"):
                self.assertEqual(paired_main(), 0)
            manifest = json.loads((output / "manifest.json").read_text())
            self.assertTrue(manifest["timing_qualified"])
            self.assertFalse(manifest["counter_lane"])
            self.assertEqual(manifest["cooperative_helper_lmr_requested"], {"A": False, "B": True})
            self.assertEqual([lane.provenance["cooperative_helper_lmr_requested"] for lane in observed], [False, True, True, False])
            self.assertTrue(all(not lane.provenance["counter_overhead_included"] for lane in observed))

    def test_policy_diagnostic_observes_changed_tree_without_identity_gate(self):
        rows = [dict(repeat=0, threads=1, position=0, side=side, depth=8,
                     nodes=100 if side == "A" else 120, bestmove="7g7f",
                     score=["cp", "12"], pv=["7g7f"], wall_nps=1000, elapsed_ms=100)
                for side in "ABBA"]
        strict = summarize(rows, fixed_depth=True)["by_threads"]["1"]
        self.assertFalse(strict["identity_pass"])
        observed = summarize(rows, fixed_depth=True, require_identity=False)["by_threads"]["1"]
        self.assertFalse(observed["fixed_depth_identity_required"])
        self.assertIsNone(observed["identity_pass"])
        self.assertEqual(observed["exact_identity_blocks"], 0)
        self.assertEqual(observed["median_paired_delta"]["nodes"], 20)

    def test_one_factor_contract_rejects_combined_flags(self):
        self.assertEqual(side_changes(["LazyFlags=15"], ["LazyFlags=4111"]), {"LazyFlags": ("15", "4111")})
        with self.assertRaises(ValueError):
            side_changes(["LazyFlags=15"], ["LazyFlags=6159"])
        with self.assertRaises(ValueError):
            side_changes(["X=0", "Y=0"], ["X=1", "Y=1"])

    def test_fixed_depth_identity_and_multithread_nondeterminism(self):
        rows = []
        for _, count, position, slot, side in schedule([1, 4], 1, 1):
            rows.append(dict(repeat=0, threads=count, position=position, side=side,
                             depth=6, nodes=100 + (slot if count == 4 else 0),
                             bestmove="7g7f", score=["cp", "12"], pv=["7g7f"], wall_nps=1000,
                             elapsed_ms=100))
        summary = summarize(rows, fixed_depth=True)["by_threads"]
        self.assertTrue(summary["1"]["identity_pass"])
        self.assertIsNone(summary["4"]["identity_pass"])
        self.assertEqual(summary["4"]["within_side_nondeterministic_blocks"], 1)
        with self.assertRaisesRegex(ValueError, "incomplete"):
            summarize(rows[:-1], fixed_depth=True)

    def test_opposite_order_depth_regressions_cannot_hide_in_block_mean(self):
        rows = [dict(repeat=0, threads=4, position=0, side=side, depth=depth,
                     nodes=100, bestmove="7g7f", score=["cp", "12"], pv=["7g7f"],
                     wall_nps=1000, elapsed_ms=100)
                for side, depth in zip("ABBA", (10, 8, 12, 10), strict=True)]
        result = summarize(rows, fixed_depth=False)["by_threads"]["4"]
        self.assertEqual(result["median_paired_delta"]["depth"], 0)
        self.assertEqual(result["directional_median_paired_delta"]["A_first"]["depth"], -2)
        self.assertEqual(result["directional_median_paired_delta"]["B_first"]["depth"], 2)
        self.assertEqual(result["raw_side_medians"]["B"]["depth"], 10)

    def test_primary_pv_score_bound_is_preserved(self):
        result = parse_search_result(["info depth 4 score cp 20 nodes 100 pv 7g7f 3c3d",
                                     "info depth 4 multipv 2 score cp 19 nodes 110 pv 2g2f"])
        self.assertEqual(result, {"score": ["cp", "20"], "pv": ["7g7f", "3c3d"]})
        self.assertEqual(parse_search_result(["info score mate 2 lowerbound nodes 1 pv 5e5d"])["score"], ["mate", "2", "lowerbound"])

    def test_extended_hint_and_retirement_records_fail_closed(self):
        values = dict.fromkeys(ROOT_HINT_TRANSFER_KEYS, 0)
        values.update(publications=3, eligible_reads=2, applied=1, first_move_beta_cuts=1,
                      researches=2, research_nodes=100, depth_gap_sum=2, depth_gap_max=2)
        def line():
            return "info string lazy_smp_root_hint " + " ".join(f"{key} {value}" for key, value in values.items())
        self.assertEqual(parse_lazy_smp_root_hint([line()], required=True), values)
        values.update(generation_gap_sum=1, generation_gap_max=1)
        with self.assertRaisesRegex(ValueError, "generations"):
            parse_lazy_smp_root_hint([line()], required=True)
        with self.assertRaisesRegex(ValueError, "invalid retirement"):
            parse_retirement(["info string lazy_smp_retirement worker 0 completed_iterations 4 retired 1 foreign_contributions 0"])

    def test_cooperative_counter_rejects_unconfirmed_raise(self):
        line = "info string cooperative_root generations 2 helper_nodes 30 helper_probes 3 helper_fail_highs 1 main_confirmations 1 confirmed_alpha_raises 1 main_nodes 0 main_tasks 0 confirmation_nodes 0 helper_completed_nodes 30 helper_aborted_nodes 0 helper_aborted_probes 0 spawn_failures 0 helpers_started 2 helper_start_delay_us 0 join_wait_us 0 alpha_gap_sum 0 alpha_gap_max 0 physical_helper_threads_started 2 physical_helper_threads_joined 2 executor_start_delay_us 0 helper_dispatch_delay_us 0"
        self.assertEqual(parse_cooperative_root([line])["main_confirmations"], 1)
        with self.assertRaisesRegex(ValueError, "exceed"):
            parse_cooperative_root([line.replace("alpha_raises 1", "alpha_raises 2")])

    def test_cooperative_counter_rejects_double_counting(self):
        base = "info string cooperative_root generations 1 helper_nodes 30 helper_probes 3 helper_fail_highs 1 main_confirmations 1 confirmed_alpha_raises 1 main_nodes 100 main_tasks 2 confirmation_nodes 20 helper_completed_nodes 25 helper_aborted_nodes 5 helper_aborted_probes 1 spawn_failures 0 helpers_started 1 helper_start_delay_us 200 join_wait_us 400 alpha_gap_sum 10 alpha_gap_max 10 physical_helper_threads_started 1 physical_helper_threads_joined 1 executor_start_delay_us 0 helper_dispatch_delay_us 0"
        self.assertEqual(parse_cooperative_root([base])["confirmation_nodes"], 20)
        for old, new in [("confirmation_nodes 20", "confirmation_nodes 101"),
                         ("helper_aborted_nodes 5", "helper_aborted_nodes 6"),
                         ("alpha_gap_max 10", "alpha_gap_max 11"),
                         ("helpers_started 1", "helpers_started 0"),
                         ("physical_helper_threads_joined 1", "physical_helper_threads_joined 0"),
                         ("physical_helper_threads_started 1 physical_helper_threads_joined 1", "physical_helper_threads_started 2 physical_helper_threads_joined 2"),
                         ("helper_fail_highs 1", "helper_fail_highs 0"),
                         ("main_confirmations 1 confirmed_alpha_raises 1", "main_confirmations 0 confirmed_alpha_raises 0")]:
            with self.subTest(new=new), self.assertRaises(ValueError):
                parse_cooperative_root([base.replace(old, new)])

        with self.assertRaisesRegex(ValueError, "total nodes"):
            parse_cooperative_root([base], total_nodes=129)
        self.assertEqual(parse_cooperative_root([base], total_nodes=130)["main_nodes"], 100)
        empty = "info string cooperative_root generations 1 helper_nodes 0 helper_probes 0 helper_fail_highs 0 main_confirmations 0 confirmed_alpha_raises 0 main_nodes 100 main_tasks 2 confirmation_nodes 0 helper_completed_nodes 0 helper_aborted_nodes 0 helper_aborted_probes 0 spawn_failures 1 helpers_started 0 helper_start_delay_us 0 join_wait_us 0 alpha_gap_sum 0 alpha_gap_max 0 physical_helper_threads_started 0 physical_helper_threads_joined 0 executor_start_delay_us 0 helper_dispatch_delay_us 0"
        self.assertEqual(parse_cooperative_root([empty])["spawn_failures"], 1)
        with self.assertRaisesRegex(ValueError, "without started"):
            parse_cooperative_root([empty.replace("join_wait_us 0", "join_wait_us 1")])

    def test_aspiration_histogram_and_retry_accounting_fail_closed(self):
        lines = ["info string root_aspiration worker 0 counts 1,1,1,100,2,0,1,1,20,0",
                 "info string root_aspiration_depth worker 0 depth 8 counts 1,1,1,100,2,0,1,1,20,0"]
        self.assertEqual(parse_aspiration(lines, total_nodes=20)[0]["totals"][2], 1)
        with self.assertRaises(ValueError):
            parse_aspiration(lines, total_nodes=19)
        for bad in [lines[:1], lines + lines[:1], lines + lines[1:],
                    [x.replace("1,1,1,100,2,0,1,1,20,0", "1,1,2,100,2,0,1,1,20,0") for x in lines],
                    [x.replace("1,1,1,100,2,0,1,1,20,0", "1,1,1,100,2,0,1,0,20,0") for x in lines],
                    [lines[0], lines[1].replace("depth 8", "depth 32")],
                    [lines[0].replace("20,0", "-20,0"), lines[1]]]:
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                parse_aspiration(bad)
        self.assertEqual(parse_aspiration([]), [])

    def test_nmp_histograms_require_exact_summary_totals(self):
        lines = ["info string v2_nmp worker 0 attempts 2 fail_high 1 verification_attempts 1 verification_successes 0 verification_overturns 1 pv_returns 0 tracking_overflow 0 prior_reductions 1",
                 "info string v2_nmp_depth worker 0 depth 5 counts 2,1,1,0,1,0,0"]
        result = parse_nmp(lines)[0]
        self.assertEqual(result["prior_reductions"], 1)
        self.assertEqual(result["by_depth"][5], [2, 1, 1, 0, 1, 0, 0])
        with self.assertRaisesRegex(ValueError, "histogram"):
            parse_nmp(lines[:1])
        with self.assertRaisesRegex(ValueError, "duplicate"):
            parse_nmp(lines + lines[1:])

    def test_pv_replay_requires_bestmove_prefix(self):
        requests = requests_for_row("startpos", {"bestmove": "7g7f", "pv": ["7g7f", "3c3d"]})
        self.assertEqual(requests[1][0], apply(START, "7g7f"))
        with self.assertRaisesRegex(ValueError, "begin"):
            requests_for_row("startpos", {"bestmove": "7g7f", "pv": ["2g2f"]})

    def test_sfen_replay_roundtrip_capture_promotion_and_drop(self):
        self.assertEqual(encode(*decode(START)), START)
        first = apply(START, "7g7f")
        self.assertEqual(first, "lnsgkgsnl/1r5b1/ppppppppp/9/9/2P6/PP1PPPPPP/1B5R1/LNSGKGSNL w - 2")
        captured = apply("4k4/9/4p4/4P4/9/9/9/9/4K4 b - 1", "5d5c+")
        self.assertEqual(captured, "4k4/9/4+P4/9/9/9/9/9/4K4 w P 2")
        dropped = apply("4k4/9/9/9/9/9/9/9/4K4 w p 1", "P*5c")
        self.assertEqual(dropped, "4k4/9/4p4/9/9/9/9/9/4K4 b - 2")


class CooperativePrestartPairingTests(unittest.TestCase):
    def test_interrupted_abba_preserves_partial_rows_and_disqualifies_timing(self):
        import json
        from pathlib import Path
        from tempfile import TemporaryDirectory
        # Exercise both ordinary exceptions and BaseException cancellation.
        for error in (RuntimeError("synthetic engine failure"), KeyboardInterrupt()):
            with self.subTest(error=type(error).__name__), TemporaryDirectory() as tmp:
                root = Path(tmp)
                engine = root / "engine"
                engine.write_bytes(b"frozen interrupted engine fixture")
                positions = root / "positions.sfen"
                positions.write_text("startpos\n")
                output = root / "out"
                args = ["paired_search_diagnostics.py", "--baseline", str(engine),
                        "--candidate", str(engine), "--positions", str(positions),
                        "--output", str(output), "--threads", "4", "--count", "1", "--repeats", "1"]
                idle = {"available": True, "busy_processes": [], "load_average": [0, 0, 0]}
                first = [dict(threads=4, depth=8, nodes=100, wall_nps=1000, elapsed_ms=100,
                              bestmove="7g7f", score=["cp", "0"], pv=["7g7f"])]
                with patch("sys.argv", args), patch("paired_search_diagnostics.host_activity", return_value=idle), patch("paired_search_diagnostics.measure", side_effect=[first, error]), patch("builtins.print"):
                    with self.assertRaises(type(error)):
                        paired_main()
                manifest = json.loads((output / "manifest.json").read_text())
                self.assertEqual(manifest["status"], "incomplete")
                self.assertEqual(manifest["completed_searches"], 1)
                self.assertFalse(manifest["timing_qualified"])
                rows = [json.loads(line) for line in (output / "rows.jsonl").read_text().splitlines()]
                self.assertEqual(len(rows), 1)
                self.assertEqual((rows[0]["side"], rows[0]["abba_slot"]), ("A", 0))
                self.assertFalse((output / "summary.json").exists())

    def test_prestart_metrics_keep_intrinsic_and_observer_names_separate(self):
        base = dict(depth=8, nodes=100, wall_nps=1000, elapsed_ms=100)
        intrinsic = dict(passes=2, first_beta_cuts=1, first_aborts=0, helper_nodes=20)
        observer = dict(helper_jobs_before_first_return=3, helper_completed_probes_before_first_return=2,
                        helper_fail_highs_before_first_return=1, first_main_nodes=15,
                        first_main_us=80, first_beta_helper_nodes=5)
        plain = work_metrics({**base, "cooperative_root_prestart": intrinsic})
        self.assertEqual(plain["coop_prestart_helper_nodes"], 20)
        self.assertNotIn("coop_prestart_diag_first_main_us", plain)
        observed = work_metrics({**base, "cooperative_root_prestart": intrinsic,
                                 "cooperative_root_prestart_diag": observer})
        self.assertEqual(observed["coop_prestart_passes"], 2)
        self.assertEqual(observed["coop_prestart_diag_helper_completed_probes_before_first_return"], 2)
        self.assertEqual(observed["coop_prestart_diag_first_main_nodes"], 15)

    def test_prestart_without_observer_retains_abba_timing_qualification(self):
        self.check_lane(common_diag=0, side_options=["--a-option", "T_COOP_PRESTART=0",
                                                    "--b-option", "T_COOP_PRESTART=1"],
                        requested={"A": False, "B": False}, qualified=True)

    def test_prestart_observer_request_excludes_timing_and_tracks_each_abba_arm(self):
        # Per-arm overrides take precedence over common options.
        self.check_lane(common_diag=1, side_options=["--a-option", "T_COOP_PRESTART_DIAG=0",
                                                    "--b-option", "T_COOP_PRESTART_DIAG=1"],
                        requested={"A": False, "B": True}, qualified=False)
        self.check_lane(common_diag=1, side_options=["--a-option", "T_COOP_PRESTART=0",
                                                    "--b-option", "T_COOP_PRESTART=1"],
                        requested={"A": True, "B": True}, qualified=False)

    def check_lane(self, *, common_diag, side_options, requested, qualified):
        import json
        from pathlib import Path
        from tempfile import TemporaryDirectory
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            engine = root / "engine"
            engine.write_bytes(b"frozen prestart engine fixture")
            positions = root / "positions.sfen"
            positions.write_text("startpos\n")
            output = root / "out"
            args = ["paired_search_diagnostics.py", "--baseline", str(engine),
                    "--candidate", str(engine), "--positions", str(positions),
                    "--output", str(output), "--threads", "4", "--count", "1", "--repeats", "1",
                    "--option", f"T_COOP_PRESTART_DIAG={common_diag}", *side_options]
            idle = {"available": True, "busy_processes": [], "load_average": [0, 0, 0]}
            observed = []
            def fake_measure(lane, threads, positions):
                observed.append(lane)
                return [dict(threads=threads, depth=8, nodes=100, wall_nps=1000, elapsed_ms=100,
                             bestmove="7g7f", score=["cp", "0"], pv=["7g7f"])]
            with patch("sys.argv", args), patch("paired_search_diagnostics.host_activity", return_value=idle), patch("paired_search_diagnostics.measure", side_effect=fake_measure), patch("builtins.print"):
                self.assertEqual(paired_main(), 0)
            manifest = json.loads((output / "manifest.json").read_text())
            self.assertEqual(manifest["status"], "complete")
            self.assertEqual(manifest["budget"], {"depth": None, "movetime_ms": 300})
            self.assertNotIn("measurement_phase", manifest)
            self.assertNotIn("cooperative_prestart_cap_requested", manifest)
            self.assertEqual(manifest["cooperative_prestart_instrumentation_requested"], requested)
            self.assertEqual(manifest["timing_qualified"], qualified)
            self.assertFalse(manifest["counter_lane"])
            abba = [requested[side] for side in "ABBA"]
            self.assertEqual([lane.provenance["cooperative_prestart_instrumentation_requested"] for lane in observed], abba)
            self.assertEqual([lane.provenance["counter_overhead_included"] for lane in observed], abba)
            self.assertTrue(all(not lane.require_lazy_diagnostics for lane in observed))


class FixedNodePairingTests(unittest.TestCase):
    def test_fixed_node_work_runs_on_idle_busy_or_unknown_host_without_identity_gate(self):
        import json
        from pathlib import Path
        from tempfile import TemporaryDirectory
        hosts = [dict(available=True, busy_processes=[]),
                 dict(available=True, busy_processes=[{"pid": 23, "cpu_percent": 831.0}]),
                 dict(available=False, busy_processes=[])]
        for host in hosts:
            with self.subTest(host=host), TemporaryDirectory() as tmp:
                root = Path(tmp)
                engine = root / "engine"
                engine.write_bytes(b"fixed-node mocked engine")
                positions = root / "positions.sfen"
                positions.write_text("startpos\n")
                output = root / "out"
                args = ["paired_search_diagnostics.py", "--baseline", str(engine), "--candidate", str(engine),
                        "--positions", str(positions), "--output", str(output), "--threads", "1", "--count", "1",
                        "--repeats", "1", "--nodes", "1000", "--a-option", "T_COOP_PRESTART=0",
                        "--b-option", "T_COOP_PRESTART=1"]
                lanes = []
                def fake_measure(lane, threads, positions):
                    lanes.append(lane)
                    different = "T_COOP_PRESTART=1" in lane.option
                    return [dict(threads=threads, depth=9 if different else 8, nodes=1008 if different else 1001,
                                 node_limit_overshoot=8 if different else 1, wall_nps=1000, elapsed_ms=100,
                                 bestmove="2g2f" if different else "7g7f", score=["cp", "1" if different else "0"],
                                 pv=["2g2f"] if different else ["7g7f"], provenance=lane.provenance)]
                with patch("sys.argv", args), patch("paired_search_diagnostics.host_activity", return_value=host), patch("paired_search_diagnostics.measure", side_effect=fake_measure), patch("builtins.print"):
                    self.assertEqual(paired_main(), 0)
                manifest = json.loads((output / "manifest.json").read_text())
                self.assertEqual(manifest["status"], "complete")
                self.assertEqual(manifest["budget"], {"depth": None, "movetime_ms": None, "node_limit": 1000})
                self.assertEqual(manifest["measurement_phase"], "fixed_node_work_diagnostic")
                self.assertEqual(manifest["identity_policy"], "observe_only_fixed_node_work")
                self.assertFalse(manifest["timing_qualified"])
                self.assertEqual(len(lanes), 4)
                self.assertTrue(all(lane.nodes == 1000 and lane.depth is None for lane in lanes))
                self.assertTrue(all(lane.provenance["timing_qualified"] is False for lane in lanes))
                summary = json.loads((output / "summary.json").read_text())["by_threads"]["1"]
                self.assertIsNone(summary["identity_pass"])
                self.assertFalse(summary["fixed_depth_identity_required"])
                self.assertEqual(summary["exact_identity_blocks"], 0)
                self.assertEqual(summary["median_paired_delta"]["node_limit_overshoot"], 7)

    def test_fixed_node_cli_rejects_budget_conflicts_and_invalid_u64_before_filesystem_changes(self):
        flags = (["--nodes", "1", "--depth", "8"], ["--nodes", "1", "--movetime", "300"],
                 ["--nodes", "0"], ["--nodes", str(1 << 64)], ["--nodes", "-1"], ["--nodes", "１"])
        for extra in flags:
            args = ["paired_search_diagnostics.py", "--baseline", "unused-A", "--candidate", "unused-B",
                    "--positions", "unused-positions", "--output", "unused-output", *extra]
            with self.subTest(flags=extra), patch("sys.argv", args), patch("sys.stderr"), patch("paired_search_diagnostics.freeze") as freeze:
                with self.assertRaises(SystemExit) as error:
                    paired_main()
                self.assertEqual(error.exception.code, 2)
                freeze.assert_not_called()


class CooperativePrestartCapPairingTests(unittest.TestCase):
    def test_cap_metrics_keep_policy_census_separate_from_observer(self):
        base = dict(depth=8, nodes=100, wall_nps=1000, elapsed_ms=100)
        cap = dict(limit=1, passes=2, initial_claims=2, park_calls=5, resumed_helpers=3, cancelled_helpers=2)
        observed = work_metrics({**base, "cooperative_root_prestart_cap": cap})
        self.assertEqual(observed["coop_prestart_cap_limit"], 1)
        self.assertEqual(observed["coop_prestart_cap_initial_claims"], 2)
        self.assertEqual(observed["coop_prestart_cap_park_calls"], 5)
        self.assertNotIn("coop_prestart_diag_first_main_us", observed)

    def test_cap_request_does_not_disqualify_time_and_node_lane_remains_unqualified(self):
        import json
        from pathlib import Path
        from tempfile import TemporaryDirectory
        for nodes, observer in ((False, False), (False, True), (True, False)):
            with self.subTest(nodes=nodes, observer=observer), TemporaryDirectory() as tmp:
                root = Path(tmp)
                engine = root / "engine"
                engine.write_bytes(b"mocked cap engine")
                positions = root / "positions.sfen"
                positions.write_text("startpos\n")
                output = root / "out"
                args = ["paired_search_diagnostics.py", "--baseline", str(engine), "--candidate", str(engine),
                        "--positions", str(positions), "--output", str(output), "--threads", "4", "--count", "1", "--repeats", "1",
                        "--option", "T_COOP_PRESTART=1", "--option", f"T_COOP_PRESTART_DIAG={int(observer)}",
                        "--a-option", "T_COOP_PRESTART_CAP=0", "--b-option", "T_COOP_PRESTART_CAP=1"]
                if nodes:
                    args += ["--nodes", "1000"]
                idle = dict(available=True, busy_processes=[], load_average=[0, 0, 0])
                lanes = []
                def fake_measure(lane, threads, positions):
                    lanes.append(lane)
                    return [dict(threads=threads, depth=8, nodes=1000, wall_nps=1000, elapsed_ms=100,
                                 bestmove="7g7f", score=["cp", "0"], pv=["7g7f"])]
                with patch("sys.argv", args), patch("paired_search_diagnostics.host_activity", return_value=idle), patch("paired_search_diagnostics.measure", side_effect=fake_measure), patch("builtins.print"):
                    self.assertEqual(paired_main(), 0)
                manifest = json.loads((output / "manifest.json").read_text())
                self.assertEqual(manifest["cooperative_prestart_cap_requested"], {"A": False, "B": True})
                self.assertEqual(manifest["timing_qualified"], not nodes and not observer)
                self.assertEqual([lane.provenance["cooperative_prestart_cap_requested"] for lane in lanes], [False, True, True, False])
                self.assertEqual([lane.provenance["counter_overhead_included"] for lane in lanes], [observer] * 4)
                self.assertEqual(manifest["budget"], {"depth": None, "movetime_ms": None, "node_limit": 1000} if nodes else {"depth": None, "movetime_ms": 300})


if __name__ == "__main__":
    unittest.main()
