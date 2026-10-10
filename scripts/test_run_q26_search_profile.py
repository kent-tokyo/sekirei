#!/usr/bin/env python3
"""Regression tests for the Q26 fixed-budget command contract."""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from run_q26_search_profile import build_command, summarize_qsearch  # noqa: E402


class SearchProfileCommandTest(unittest.TestCase):
    def test_fixed_nodes_does_not_enable_fixed_depth_mode(self):
        command = build_command(
            Path("search"),
            {"sfen": "position"},
            100_000,
            None,
            50,
            None,
            True,
        )

        self.assertIn("--nodes", command)
        self.assertNotIn("--max-depth", command)
        self.assertIn("--profile-cost", command)
        self.assertNotIn("--weights", command)

    def test_fixed_time_keeps_the_requested_depth_cap_and_weights(self):
        command = build_command(
            Path("search"),
            {"sfen": "position"},
            100_000,
            250,
            32,
            Path("weights.bin"),
            False,
        )

        self.assertIn("--time-ms", command)
        self.assertIn("--max-depth", command)
        self.assertIn("32", command)
        self.assertIn("--weights", command)
        self.assertNotIn("--profile-cost", command)

    def test_qsearch_summary_pools_counts_before_computing_rates(self):
        profiles = [
            {
                "quiescence_calls": 10,
                "qsearch_top_level_calls": 4,
                "qsearch_in_check": 2,
                "qsearch_tt_cutoffs": 1,
                "qsearch_stand_pat_cutoffs": 4,
                "qsearch_terminal_nodes": 1,
                "qsearch_searched_moves": 8,
                "qsearch_beta_cutoffs": 2,
                "qsearch_depth_cap_exits": 0,
                "qsearch_mate_in_one_exits": 1,
                "qsearch_delta_pruning_exits": 0,
            },
            {
                "quiescence_calls": 30,
                "qsearch_top_level_calls": 6,
                "qsearch_in_check": 8,
                "qsearch_tt_cutoffs": 3,
                "qsearch_stand_pat_cutoffs": 6,
                "qsearch_terminal_nodes": 2,
                "qsearch_searched_moves": 32,
                "qsearch_beta_cutoffs": 8,
                "qsearch_depth_cap_exits": 1,
                "qsearch_mate_in_one_exits": 0,
                "qsearch_delta_pruning_exits": 1,
            },
        ]

        summary = summarize_qsearch(profiles)

        self.assertEqual(summary["totals"]["quiescence_calls"], 40)
        self.assertEqual(summary["searched_moves_per_call"], 1.0)
        self.assertEqual(summary["in_check_share"], 0.25)
        self.assertEqual(summary["tt_cutoff_share"], 0.1)
        self.assertEqual(summary["tt_cutoffs_per_top_level_call"], 0.4)
        self.assertEqual(summary["stand_pat_cutoff_share"], 0.25)
        self.assertEqual(summary["beta_cutoff_per_searched_move"], 0.25)


if __name__ == "__main__":
    unittest.main()
