#!/usr/bin/env python3
"""Tests for Q21v transfer-audit metrics."""

from __future__ import annotations

import unittest

import run_q21v_transfer_audit as q21v


class Q21vTests(unittest.TestCase):
    def test_teacher_distribution_deduplicates_top_move(self) -> None:
        corpus = {
            "pairs": [
                {
                    "parent_id": "p1",
                    "category": "opening/balanced",
                    "higher_move_usi": "7g7f",
                    "lower_move_usi": "2g2f",
                    "teacher_score_gap_cp": 100,
                },
                {
                    "parent_id": "p1",
                    "category": "opening/balanced",
                    "higher_move_usi": "7g7f",
                    "lower_move_usi": "5i6h",
                    "teacher_score_gap_cp": 300,
                },
            ]
        }
        result = q21v.teacher_distribution(corpus)
        self.assertEqual(result["candidate_count"]["mean"], 3)
        self.assertEqual(result["nonzero_teacher_gap_cp"]["median"], 200)

    def test_score_transfer_counts_quantized_movement(self) -> None:
        before = {
            "model_diagnostic": {
                "move_diagnostics": [
                    {"parent_id": "p", "move_usi": "a", "parent_score_cp": 10, "teacher_top": True},
                    {"parent_id": "p", "move_usi": "b", "parent_score_cp": 5, "teacher_top": False},
                ]
            }
        }
        after = {
            "model_diagnostic": {
                "move_diagnostics": [
                    {"parent_id": "p", "move_usi": "a", "parent_score_cp": 12, "teacher_top": True},
                    {"parent_id": "p", "move_usi": "b", "parent_score_cp": 4, "teacher_top": False},
                ]
            }
        }
        result = q21v.score_transfer(before, after)
        self.assertEqual(result["changed_moves"], 2)
        self.assertEqual(result["mean_abs_score_delta_cp"], 1.5)


if __name__ == "__main__":
    unittest.main()
