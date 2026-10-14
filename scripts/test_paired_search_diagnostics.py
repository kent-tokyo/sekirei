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


if __name__ == "__main__":
    unittest.main()
