import unittest
from unittest.mock import patch
from types import SimpleNamespace

from paired_search_diagnostics import schedule, side_changes, summarize, host_activity
from generate_search_diagnostic import START, apply, decode, encode
from nps_threads import ROOT_HINT_TRANSFER_KEYS, parse_lazy_smp_root_hint, parse_retirement, parse_search_result, parse_nmp


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
