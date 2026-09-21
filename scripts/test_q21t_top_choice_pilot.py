#!/usr/bin/env python3
"""Unit tests for Q21t's direct-regret decision helpers."""

from __future__ import annotations

import unittest

import run_q21t_top_choice_pilot as q21t


class Q21tMetricsTests(unittest.TestCase):
    def test_static_metrics_uses_direct_parent_regret(self) -> None:
        audit = {
            "parent_positions": 3,
            "model_diagnostic": {
                "mean_parent_rank_loss_cp": 150.0,
                "teacher_preferred_ordering_rate": 0.75,
                "parent_diagnostics": [
                    {"teacher_rank_loss_cp": 0},
                    {"teacher_rank_loss_cp": 150},
                    {"teacher_rank_loss_cp": 300},
                ],
            },
        }
        metrics = q21t.static_metrics(audit)
        self.assertEqual(metrics["top1_matches"], 1)
        self.assertEqual(metrics["major_regrets_ge_300cp"], 1)
        self.assertEqual(metrics["mean_direct_top_regret_cp"], 150.0)

    def test_same_time_metrics_requires_stable_move(self) -> None:
        rows = [
            {
                "arm": "candidate",
                "parent_id": "p1",
                "result": {"bestmove": "7g7f"},
                "teacher_direct_top_regret_cp": 0,
            },
            {
                "arm": "candidate",
                "parent_id": "p1",
                "result": {"bestmove": "2g2f"},
                "teacher_direct_top_regret_cp": 20,
            },
        ]
        metrics = q21t.same_time_metrics(rows, "candidate")
        self.assertEqual(metrics["stable_parents"], 0)
        self.assertEqual(metrics["comparable_parents"], 0)
        self.assertIsNone(metrics["mean_direct_top_regret_cp"])


if __name__ == "__main__":
    unittest.main()
