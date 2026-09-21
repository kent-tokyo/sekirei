#!/usr/bin/env python3
"""Tests for Q21u train-only calibration and metrics."""

from __future__ import annotations

import unittest

import prepare_q21u_listwise_pilot as prepare
import run_q21u_train_screen as screen
import run_q21u_validation_screen as validation


class Q21uTests(unittest.TestCase):
    def test_softmax_is_normalized_and_monotonic(self) -> None:
        values = prepare.softmax({"a": 100, "b": 0, "c": -100}, 400)
        self.assertAlmostEqual(sum(values.values()), 1.0)
        self.assertGreater(values["a"], values["b"])
        self.assertGreater(values["b"], values["c"])

    def test_metrics_use_direct_parent_regret(self) -> None:
        audit = {
            "model_diagnostic": {
                "mean_parent_rank_loss_cp": 100.0,
                "teacher_preferred_ordering_rate": 0.5,
                "parent_diagnostics": [
                    {"teacher_rank_loss_cp": 0},
                    {"teacher_rank_loss_cp": 300},
                ],
            }
        }
        result = screen.metrics(audit)
        self.assertEqual(result["top1_matches"], 1)
        self.assertEqual(result["major_regrets_ge_300cp"], 1)

    def test_mate_like_early_completion_is_recordable_but_not_ordinary(self) -> None:
        result = {
            "completion": "search_completed",
            "completed_bound": "exact",
            "completed_iteration_valid": "true",
            "aborted": False,
            "abort_reason": "none",
            "pv_legal": True,
            "pv_replay_preserves_input": True,
            "history_matches_expected": "true",
            "depth": 1,
            "score_cp": -899_998,
        }
        self.assertTrue(validation.exact_depth7_or_terminal(result))
        result["score_cp"] = -900
        self.assertFalse(validation.exact_depth7_or_terminal(result))


if __name__ == "__main__":
    unittest.main()
