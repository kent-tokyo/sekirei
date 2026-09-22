#!/usr/bin/env python3
"""Regression tests for terminal-root handling in external label generation."""

from __future__ import annotations

import unittest

import run_q25a_external_labels as labels


class ShallowCandidateCompletionTests(unittest.TestCase):
    def test_terminal_primary_accepts_complete_root_candidate_set(self) -> None:
        result = {
            "completion": "search_completed",
            "completed_bound": "exact",
            "depth": 1,
            "root_candidates": [
                {"move": "7g7f", "score_cp": 100, "depth": 3, "bound": "exact", "abort_reason": "none"},
                {"move": "2g2f", "score_cp": 90, "depth": 3, "bound": "exact", "abort_reason": "none"},
            ],
        }
        self.assertTrue(labels.shallow_candidates_complete(result, 3))

    def test_incomplete_root_candidate_set_stays_invalid(self) -> None:
        result = {
            "completion": "search_completed",
            "completed_bound": "exact",
            "depth": 1,
            "root_candidates": [
                {"move": "7g7f", "score_cp": 100, "depth": 3, "bound": "exact", "abort_reason": "none"},
                {"move": "2g2f", "score_cp": 90, "depth": 2, "bound": "exact", "abort_reason": "none"},
            ],
        }
        self.assertFalse(labels.shallow_candidates_complete(result, 3))

    def test_depth_complete_single_legal_move_stays_invalid(self) -> None:
        result = {
            "completion": "search_completed",
            "completed_bound": "exact",
            "depth": 3,
            "root_candidates": [
                {"move": "7g7f", "score_cp": 100, "depth": 3, "bound": "exact", "abort_reason": "none"},
            ],
        }
        self.assertFalse(labels.shallow_candidates_complete(result, 3))

    def test_terminal_root_candidates_exclude_partial_depth_one_scores(self) -> None:
        result = {
            "root_candidates": [
                {"move": "7g7f", "score_cp": 99_999, "depth": 1, "bound": "exact", "abort_reason": "none"},
                {"move": "2g2f", "score_cp": 100, "depth": 3, "bound": "exact", "abort_reason": "none"},
                {"move": "3g3f", "score_cp": 90, "depth": 3, "bound": "exact", "abort_reason": "none"},
            ]
        }
        self.assertEqual(labels.shallow_top_moves(result, 2, 3), ["2g2f", "3g3f"])


if __name__ == "__main__":
    unittest.main()
