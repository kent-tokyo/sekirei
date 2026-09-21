#!/usr/bin/env python3
"""Regression coverage for diagnostic corpus shape normalization."""
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(__file__))

from classify_diagnostic_positions import corpus_positions
from diagnostic_contract import stable_entry_id


class CorpusPositionsTest(unittest.TestCase):
    def test_history_aware_positions_keep_explicit_id(self):
        corpus = {
            "diagnostic_only": True,
            "positions": [{"id": "holdout-01", "sfen": "board b - 1", "category": "opening"}],
        }
        self.assertEqual(corpus_positions(corpus), [{
            "id": "holdout-01", "sfen": "board b - 1", "source_category": "opening",
        }])

    def test_csa_replay_entries_receive_stable_game_ply_id(self):
        entry = {
            "source": {"game_id": "game-a", "ply": 7},
            "position": {"sfen": "board w P 8"},
            "selection_reason": "opponent_capture",
        }
        corpus = {
            "diagnostic_only": True,
            "entries": [entry],
        }
        self.assertEqual(corpus_positions(corpus), [{
            "id": stable_entry_id(entry), "sfen": "board w P 8", "source_category": "opponent_capture",
        }])

    def test_failure_audit_positions_accept_nested_sfen(self):
        corpus = {
            "diagnostic_only": True,
            "positions": [{
                "id": "q21j-game-01",
                "selection": {"rule": "first_candidate_score_drop_ge_300cp"},
                "position": {"sfen": "board b - 42"},
            }],
        }
        self.assertEqual(corpus_positions(corpus), [{
            "id": "q21j-game-01",
            "sfen": "board b - 42",
            "source_category": "first_candidate_score_drop_ge_300cp",
        }])

    def test_non_diagnostic_corpus_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "diagnostic-only"):
            corpus_positions({"entries": []})


if __name__ == "__main__":
    unittest.main()
