#!/usr/bin/env python3
"""Unit tests for explicit evaluator and thread-scaling options."""

import argparse
import io
import subprocess
import sys
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))

from nps_threads import (  # noqa: E402
    ELAPSED_SCOPE,
    Usi,
    engine_options,
    format_optional_ratio,
    go_command,
    measure,
    parse_bestmove,
    parse_cooperative_root_wait,
    parse_lazy_smp,
    parse_lazy_smp_overlap,
    parse_lazy_smp_root_hint,
    parse_lazy_smp_tt,
    require_lazy_diagnostic_record,
    summarize_lazy_overlap,
    summarize_lazy_root_hints,
    summarize_lazy_root_work,
    summarize_lazy_tt,
    summarize_selected_tt_reuse,
)


class ThreadOptionsTest(unittest.TestCase):
    @staticmethod
    def args(*, yaneuraou: bool, evalfile: str | None) -> argparse.Namespace:
        return argparse.Namespace(
            yaneuraou=yaneuraou,
            evalfile=evalfile,
            fv_scale=24,
            hash=64,
            option=[],
            lazy_diagnostics=False,
        )

    def test_material_only_sekirei_omits_evalfile(self):
        options = engine_options(self.args(yaneuraou=False, evalfile=None), 4)

        self.assertIn("Threads=4", options)
        self.assertFalse(any(option.startswith("EvalFile=") for option in options))

    def test_explicit_sekirei_evalfile_is_preserved(self):
        options = engine_options(self.args(yaneuraou=False, evalfile="nn.bin"), 2)

        self.assertIn(f"EvalFile={Path('nn.bin').resolve()}", options)

    def test_relative_engine_path_is_resolved_before_changing_directory(self):
        with patch("nps_threads.subprocess.Popen") as popen:
            Usi("target/release/sekirei", {})

        binary = Path("target/release/sekirei").resolve()
        self.assertEqual(popen.call_args.args[0], [str(binary)])
        self.assertEqual(popen.call_args.kwargs["cwd"], str(binary.parent))

    def test_quit_reaps_an_engine_that_does_not_exit_in_time(self):
        with patch("nps_threads.subprocess.Popen") as popen:
            process = popen.return_value
            process.wait.side_effect = [
                subprocess.TimeoutExpired("sekirei", 5),
                0,
            ]
            engine = Usi("target/release/sekirei", {})
            engine.quit()

        process.kill.assert_called_once_with()
        self.assertEqual(process.wait.call_count, 2)

    def test_wait_for_collects_response_and_detects_early_exit(self):
        with patch("nps_threads.subprocess.Popen") as popen:
            popen.return_value.stdout = io.StringIO("info depth 1\nbestmove 7g7f\n")
            engine = Usi("engine", {}, timeout=1)
            self.assertEqual(engine.wait_for("bestmove"), ["info depth 1", "bestmove 7g7f"])
            with self.assertRaisesRegex(RuntimeError, "exited"):
                engine.wait_for("readyok")

    def test_position_ready_barrier_excludes_pending_reset_on_every_repeat(self):
        for reported_nps in (98765, 0):
            with self.subTest(reported_nps=reported_nps):
                args = self.args(yaneuraou=False, evalfile=None)
                args.engine = "fake-engine"
                args.repeats = 2
                args.movetime = 1000
                args.depth = None
                args.require_lazy_diagnostics = False
                args.require_root_hint_diagnostics = False
                args.provenance = {"counter_overhead_included": False}
                clock = 0.0
                pending_reset = 0.0
                events = []

                def send(line):
                    nonlocal pending_reset
                    events.append(("send", line))
                    if line == "usinewgame":
                        pending_reset += 0.5

                def wait_for(token):
                    nonlocal clock, pending_reset
                    events.append(("wait", token))
                    # A queued reset is processed before either readyok or
                    # bestmove; omitting the barrier would time an extra 500ms.
                    clock += pending_reset
                    pending_reset = 0.0
                    if token == "bestmove":
                        clock += 0.125
                        return [f"info depth 4 score cp 0 nodes 900 nps {reported_nps} pv 7g7f",
                                "bestmove 7g7f"]
                    return [token]

                def now():
                    events.append(("clock", clock))
                    return clock

                with patch("nps_threads.Usi") as usi, patch("nps_threads.time.perf_counter", side_effect=now), patch("nps_threads.sys.stderr"):
                    usi.return_value.send.side_effect = send
                    usi.return_value.wait_for.side_effect = wait_for
                    rows = measure(args, 4, ["startpos", "startpos moves 7g7f"])
                    usi.return_value.quit.assert_called_once_with()

                self.assertEqual(len(rows), 4)
                self.assertEqual([row["elapsed_ms"] for row in rows], [125] * 4)
                self.assertEqual([row["nps"] for row in rows],
                                 [reported_nps or 7200] * 4)
                self.assertTrue(all(row["provenance"]["elapsed_scope"] == ELAPSED_SCOPE
                                    for row in rows))
                for index, event in enumerate(events):
                    if event == ("send", "go movetime 1000"):
                        self.assertEqual(events[index - 1][0], "clock")
                        self.assertEqual(events[index - 2], ("wait", "readyok"))
                        self.assertEqual(events[index - 3], ("send", "isready"))
                        self.assertTrue(events[index - 4][1].startswith("position "))
                self.assertEqual(events.count(("wait", "readyok")), 5)

    def test_lazy_smp_diagnostics_are_parsed(self):
        diagnostics = parse_lazy_smp(
            [
                "info string lazy_smp selected w2 node_share_permille 268 "
                "move_agreement 3/4 discarded_root_nodes 33 "
                "divergent_discarded_nodes 10 stop_lag_ms 2 "
                "selected_tt_local_hits 7 selected_tt_foreign_hits 5 "
                "selected_tt_unknown_hits 2 "
                "w0:d20:n10:s1:t300:x1:aexternal:p30:h20:r10:l8:f3:u1,"
                "w1:d20:n11:s2:t301:x1:aexternal:p31:h21:r11:l9:f4:u1,"
                "w2*:d21:n12:s3:t302:x0:acompleted:p32:h22:r12:l7:f5:u2,"
                "w3:d20:n12:s3:t301:x1:aexternal:p33:h23:r13:l9:f6:u1"
            ]
        )

        self.assertEqual(
            diagnostics,
            {
                "selected_worker": 2,
                "selected_node_share_permille": 268,
                "move_agreement": 3,
                "workers": 4,
                "discarded_root_nodes": 33,
                "divergent_discarded_nodes": 10,
                "stop_lag_ms": 2,
                "selected_tt_local_hits": 7,
                "selected_tt_foreign_hits": 5,
                "selected_tt_unknown_hits": 2,
                "worker_stats": [
                    {
                        "worker": 0,
                        "selected": False,
                        "depth": 20,
                        "nodes": 10,
                        "score": 1,
                        "elapsed_ms": 300,
                        "aborted": True,
                        "abort_reason": "external",
                        "tt_probes": 30,
                        "tt_hits": 20,
                        "tt_stores": 10,
                        "tt_local_hits": 8,
                        "tt_foreign_hits": 3,
                        "tt_unknown_hits": 1,
                    },
                    {
                        "worker": 1,
                        "selected": False,
                        "depth": 20,
                        "nodes": 11,
                        "score": 2,
                        "elapsed_ms": 301,
                        "aborted": True,
                        "abort_reason": "external",
                        "tt_probes": 31,
                        "tt_hits": 21,
                        "tt_stores": 11,
                        "tt_local_hits": 9,
                        "tt_foreign_hits": 4,
                        "tt_unknown_hits": 1,
                    },
                    {
                        "worker": 2,
                        "selected": True,
                        "depth": 21,
                        "nodes": 12,
                        "score": 3,
                        "elapsed_ms": 302,
                        "aborted": False,
                        "abort_reason": "completed",
                        "tt_probes": 32,
                        "tt_hits": 22,
                        "tt_stores": 12,
                        "tt_local_hits": 7,
                        "tt_foreign_hits": 5,
                        "tt_unknown_hits": 2,
                    },
                    {
                        "worker": 3,
                        "selected": False,
                        "depth": 20,
                        "nodes": 12,
                        "score": 3,
                        "elapsed_ms": 301,
                        "aborted": True,
                        "abort_reason": "external",
                        "tt_probes": 33,
                        "tt_hits": 23,
                        "tt_stores": 13,
                        "tt_local_hits": 9,
                        "tt_foreign_hits": 6,
                        "tt_unknown_hits": 1,
                    },
                ],
            },
        )

    def test_malformed_lazy_smp_diagnostics_are_ignored(self):
        self.assertIsNone(parse_lazy_smp(["info string lazy_smp selected nope"]))

    def test_lazy_smp_tt_diagnostics_are_parsed(self):
        diagnostics = parse_lazy_smp_tt(
            [
                "info string lazy_smp_tt probes 100 hits 61 attempted 40 "
                "committed 35 same_hash 20 equal_depth 4 shallower_rejected 3 "
                "collision_overwrites 11"
            ]
        )

        self.assertEqual(
            diagnostics,
            {
                "probes": 100,
                "hits": 61,
                "attempted": 40,
                "committed": 35,
                "same_hash": 20,
                "equal_depth": 4,
                "shallower_rejected": 3,
                "collision_overwrites": 11,
            },
        )

    def test_lazy_smp_overlap_diagnostics_are_parsed(self):
        diagnostics = parse_lazy_smp_overlap(
            [
                "info string lazy_smp_overlap sample_rate 256 visits 100 "
                "unique 40 within_worker_revisits 35 cross_worker_visits 25 "
                "shared_fingerprints 12 dropped 0 tt_sampled_stores 50 "
                "tt_sampled_hits 30 tt_local_hits 18 tt_foreign_hits 9 "
                "tt_unknown_hits 3 tt_dropped_stores 1"
            ]
        )

        self.assertEqual(
            diagnostics,
            {
                "sample_rate": 256,
                "visits": 100,
                "unique": 40,
                "within_worker_revisits": 35,
                "cross_worker_visits": 25,
                "shared_fingerprints": 12,
                "dropped": 0,
                "tt_sampled_stores": 50,
                "tt_sampled_hits": 30,
                "tt_local_hits": 18,
                "tt_foreign_hits": 9,
                "tt_unknown_hits": 3,
                "tt_dropped_stores": 1,
            },
        )

    def test_root_hint_diagnostics_are_parsed(self):
        self.assertEqual(
            parse_lazy_smp_root_hint(
                [
                    "info string lazy_smp_root_hint publications 7 "
                    "eligible_reads 5 applied 3"
                ],
                required=True,
            ),
            {"publications": 7, "eligible_reads": 5, "applied": 3},
        )

    def test_required_root_hint_diagnostic_fails_closed(self):
        with self.assertRaisesRegex(ValueError, "missing"):
            parse_lazy_smp_root_hint([], required=True)
        with self.assertRaisesRegex(ValueError, "expected one"):
            parse_lazy_smp_root_hint(
                [
                    "info string lazy_smp_root_hint publications 1 eligible_reads 1 applied 1",
                    "info string lazy_smp_root_hint publications 1 eligible_reads 1 applied 1",
                ],
                required=True,
            )
        with self.assertRaisesRegex(ValueError, "applications exceed"):
            parse_lazy_smp_root_hint(
                [
                    "info string lazy_smp_root_hint publications 1 "
                    "eligible_reads 1 applied 2"
                ],
                required=True,
            )
        with self.assertRaisesRegex(ValueError, "without a publication"):
            parse_lazy_smp_root_hint(
                [
                    "info string lazy_smp_root_hint publications 0 "
                    "eligible_reads 1 applied 0"
                ],
                required=True,
            )
        with self.assertRaisesRegex(ValueError, "malformed"):
            parse_lazy_smp_root_hint(
                [
                    "info string lazy_smp_root_hint eligible_reads 1 "
                    "publications 1 applied 0"
                ],
                required=True,
            )
        with self.assertRaisesRegex(ValueError, "unsigned decimal"):
            parse_lazy_smp_root_hint(
                [
                    "info string lazy_smp_root_hint publications +1 "
                    "eligible_reads 1 applied 0"
                ],
                required=True,
            )
        with self.assertRaisesRegex(ValueError, "exceeds u64"):
            parse_lazy_smp_root_hint(
                [
                    f"info string lazy_smp_root_hint publications {1 << 64} "
                    "eligible_reads 0 applied 0"
                ],
                required=True,
            )

    def test_bestmove_parser_uses_terminal_response(self):
        self.assertEqual(
            parse_bestmove(["info depth 4 pv 7g7f", "bestmove 7g7f ponder 3c3d"]),
            "7g7f",
        )

    def test_lazy_diagnostics_option_is_explicit(self):
        args = self.args(yaneuraou=False, evalfile=None)
        args.lazy_diagnostics = True

        self.assertIn("LazyDiagnostics=true", engine_options(args, 4))

    def test_required_lazy_diagnostics_enable_the_engine_counters(self):
        args = self.args(yaneuraou=False, evalfile=None)
        args.require_lazy_diagnostics = True

        self.assertIn("LazyDiagnostics=true", engine_options(args, 4))

    def test_required_lazy_diagnostics_fail_on_partial_records(self):
        complete = {
            "worker_stats": [{"worker": 0, "selected": True}],
            "selected_tt_local_hits": 1,
            "selected_tt_foreign_hits": 2,
            "selected_tt_unknown_hits": 0,
        }
        require_lazy_diagnostic_record(complete, {"probes": 3}, {"visits": 4})

        with self.assertRaisesRegex(ValueError, "selected_tt_foreign_hits"):
            require_lazy_diagnostic_record(
                {key: value for key, value in complete.items() if key != "selected_tt_foreign_hits"},
                {"probes": 3},
                {"visits": 4},
            )
        with self.assertRaisesRegex(ValueError, "lazy_smp_overlap"):
            require_lazy_diagnostic_record(complete, {"probes": 3}, None)

    def test_cooperative_wait_evidence_is_optional_and_bounded(self):
        root = {"main_confirmations": 3, "confirmation_nodes": 100, "confirmed_alpha_raises": 2}
        line = ("info string cooperative_root_wait enabled 1 waiting_confirmations 2 "
                "waiting_confirmation_nodes 80 waiting_alpha_raises 1 notification_wait_us 50")
        self.assertIsNone(parse_cooperative_root_wait([], cooperative_root=None))
        parsed = parse_cooperative_root_wait([line], cooperative_root=root)
        self.assertEqual(parsed, {"enabled": 1, "waiting_confirmations": 2,
                                 "waiting_confirmation_nodes": 80,
                                 "waiting_alpha_raises": 1, "notification_wait_us": 50})
        for bad in ([line, line], [line + " extra 0"], [line.replace("enabled 1", "enabled 0")],
                    [line.replace("enabled 1", "enabled 2")],
                    [line.replace("waiting_confirmations 2", "waiting_confirmations 4")],
                    [line.replace("waiting_confirmation_nodes 80", "waiting_confirmation_nodes 101")],
                    [line.replace("waiting_alpha_raises 1", "waiting_alpha_raises 3")],
                    [line.replace("waiting_confirmations 2", "waiting_confirmations 0")],
                    [line.replace("notification_wait_us 50", "notification_wait_us -1")],
                    [line.replace("notification_wait_us 50", "notification_wait_us ５０")],
                    [line.replace("notification_wait_us 50", f"notification_wait_us {1 << 64}")]):
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                parse_cooperative_root_wait(bad, cooperative_root=root)
        with self.assertRaisesRegex(ValueError, "requires enabled split"):
            parse_cooperative_root_wait([line], cooperative_root=None)
        with self.assertRaisesRegex(ValueError, "confirmed raises"):
            parse_cooperative_root_wait([line], cooperative_root={**root, "confirmed_alpha_raises": 0})

    def test_go_command_keeps_time_and_fixed_depth_budgets_distinct(self):
        self.assertEqual(
            go_command(argparse.Namespace(depth=None, movetime=300)), "go movetime 300"
        )
        self.assertEqual(
            go_command(argparse.Namespace(depth=12, movetime=300)), "go depth 12"
        )

    def test_tt_summary_pools_counts_before_computing_rates(self):
        summary = summarize_lazy_tt(
            [
                {
                    "lazy_smp_tt": {
                        "probes": 10,
                        "hits": 4,
                        "attempted": 5,
                        "committed": 4,
                        "same_hash": 2,
                        "equal_depth": 0,
                        "shallower_rejected": 1,
                        "collision_overwrites": 1,
                    }
                },
                {
                    "lazy_smp_tt": {
                        "probes": 30,
                        "hits": 16,
                        "attempted": 15,
                        "committed": 14,
                        "same_hash": 8,
                        "equal_depth": 0,
                        "shallower_rejected": 1,
                        "collision_overwrites": 0,
                    }
                },
            ]
        )

        self.assertIsNotNone(summary)
        assert summary is not None
        self.assertEqual(summary["probes"], 40)
        self.assertEqual(summary["hits"], 20)
        self.assertEqual(summary["hit_rate"], 0.5)
        self.assertEqual(summary["same_hash_rate"], 0.5)
        self.assertEqual(summary["collision_rate"], 0.05)

    def test_overlap_summary_pools_counts_before_computing_rates(self):
        summary = summarize_lazy_overlap(
            [
                {
                    "lazy_smp_overlap": {
                        "sample_rate": 256,
                        "visits": 40,
                        "unique": 20,
                        "within_worker_revisits": 10,
                        "cross_worker_visits": 10,
                        "shared_fingerprints": 5,
                        "dropped": 0,
                        "tt_sampled_stores": 20,
                        "tt_sampled_hits": 10,
                        "tt_local_hits": 6,
                        "tt_foreign_hits": 3,
                        "tt_unknown_hits": 1,
                        "tt_dropped_stores": 0,
                    }
                },
                {
                    "lazy_smp_overlap": {
                        "sample_rate": 256,
                        "visits": 60,
                        "unique": 30,
                        "within_worker_revisits": 12,
                        "cross_worker_visits": 18,
                        "shared_fingerprints": 9,
                        "dropped": 1,
                        "tt_sampled_stores": 30,
                        "tt_sampled_hits": 20,
                        "tt_local_hits": 9,
                        "tt_foreign_hits": 8,
                        "tt_unknown_hits": 3,
                        "tt_dropped_stores": 1,
                    }
                },
            ]
        )

        self.assertIsNotNone(summary)
        assert summary is not None
        self.assertEqual(summary["visits"], 100)
        self.assertEqual(summary["unique"], 50)
        self.assertEqual(summary["cross_worker_visit_rate"], 0.28)
        self.assertEqual(summary["shared_fingerprint_rate"], 0.28)
        self.assertEqual(summary["within_worker_revisit_rate"], 0.22)
        self.assertEqual(summary["drop_rate"], 0.01)
        self.assertEqual(summary["foreign_tt_hit_rate"], 11 / 26)
        self.assertEqual(summary["tt_unknown_hit_rate"], 4 / 30)

    def test_root_work_summary_uses_total_worker_nodes(self):
        summary = summarize_lazy_root_work(
            [
                {
                    "nodes": 100,
                    "lazy_smp": {
                        "discarded_root_nodes": 70,
                        "divergent_discarded_nodes": 40,
                    },
                },
                {
                    "nodes": 300,
                    "lazy_smp": {
                        "discarded_root_nodes": 180,
                        "divergent_discarded_nodes": 60,
                    },
                },
                {"nodes": 999},
            ]
        )

        self.assertIsNotNone(summary)
        assert summary is not None
        self.assertEqual(summary["total_nodes"], 400)
        self.assertEqual(summary["selected_root_nodes"], 150)
        self.assertEqual(summary["discarded_root_node_rate"], 0.625)
        self.assertEqual(summary["divergent_discarded_node_rate"], 0.25)

    def test_selected_tt_reuse_summary_excludes_unselected_workers(self):
        summary = summarize_selected_tt_reuse(
            [
                {
                    "lazy_smp": {
                        "selected_tt_local_hits": 6,
                        "selected_tt_foreign_hits": 3,
                        "selected_tt_unknown_hits": 1,
                    }
                },
                {
                    "lazy_smp": {
                        "selected_tt_local_hits": 9,
                        "selected_tt_foreign_hits": 7,
                        "selected_tt_unknown_hits": 4,
                    }
                },
                {"lazy_smp": {"discarded_root_nodes": 123}},
            ]
        )

        self.assertIsNotNone(summary)
        assert summary is not None
        self.assertEqual(summary["selected_tt_local_hits"], 15)
        self.assertEqual(summary["selected_tt_foreign_hits"], 10)
        self.assertEqual(summary["selected_tt_unknown_hits"], 5)
        self.assertEqual(summary["selected_foreign_tt_hit_rate"], 0.4)
        self.assertEqual(summary["selected_tt_unknown_hit_rate"], 1 / 6)

    def test_root_hint_summary_pools_counts_before_rates(self):
        summary = summarize_lazy_root_hints(
            [
                {
                    "lazy_smp_root_hint": {
                        "publications": 2,
                        "eligible_reads": 3,
                        "applied": 1,
                    }
                },
                {
                    "lazy_smp_root_hint": {
                        "publications": 0,
                        "eligible_reads": 0,
                        "applied": 0,
                    }
                },
            ]
        )

        self.assertIsNotNone(summary)
        assert summary is not None
        self.assertEqual(summary["searches"], 2)
        self.assertEqual(summary["publications"], 2)
        self.assertEqual(summary["eligible_reads"], 3)
        self.assertEqual(summary["applied"], 1)
        self.assertEqual(summary["publication_search_rate"], 0.5)
        self.assertEqual(summary["application_search_rate"], 0.5)
        self.assertEqual(summary["reads_per_publication"], 1.5)
        self.assertEqual(summary["application_rate"], 1 / 3)

    def test_root_hint_zero_denominators_are_not_reported_as_zero_rates(self):
        summary = summarize_lazy_root_hints(
            [
                {
                    "lazy_smp_root_hint": {
                        "publications": 0,
                        "eligible_reads": 0,
                        "applied": 0,
                    }
                }
            ]
        )

        self.assertIsNotNone(summary)
        assert summary is not None
        self.assertIsNone(summary["reads_per_publication"])
        self.assertIsNone(summary["application_rate"])
        self.assertEqual(format_optional_ratio(None, percentage=True), "n/a")

    def test_required_root_hint_summary_rejects_partial_rows(self):
        with self.assertRaisesRegex(ValueError, "missing diagnostic"):
            summarize_lazy_root_hints(
                [
                    {
                        "lazy_smp_root_hint": {
                            "publications": 1,
                            "eligible_reads": 1,
                            "applied": 1,
                        }
                    },
                    {},
                ],
                require_all=True,
            )


if __name__ == "__main__":
    unittest.main()
