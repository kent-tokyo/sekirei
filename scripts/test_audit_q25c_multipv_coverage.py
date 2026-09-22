#!/usr/bin/env python3
"""Pure-data tests for Q25c wider-MultiPV coverage accounting."""

from __future__ import annotations

import unittest

import audit_q25c_multipv_coverage as audit


class CoverageAuditTests(unittest.TestCase):
    def test_keeps_only_aa_stable_ordinary_candidate_scores(self) -> None:
        results = [
            {"completion": "search_completed", "bestmove": "7g7f", "lines": [
                {"move": "7g7f", "score": {"kind": "cp", "value": 20}},
                {"move": "2g2f", "score": {"kind": "cp", "value": -10}},
                {"move": "3g3f", "score": {"kind": "mate", "value": 3}},
            ]},
            {"completion": "search_completed", "bestmove": "7g7f", "lines": [
                {"move": "7g7f", "score": {"kind": "cp", "value": 20}},
                {"move": "2g2f", "score": {"kind": "cp", "value": -10}},
                {"move": "3g3f", "score": {"kind": "mate", "value": 3}},
            ]},
        ]
        labels = audit.stable_labels(results, ["7g7f", "2g2f", "3g3f", "4g4f"], 10_000)
        self.assertEqual(labels, {"7g7f": {"kind": "cp", "value": 20}, "2g2f": {"kind": "cp", "value": -10}})

    def test_rejects_changed_aa_score(self) -> None:
        results = [
            {"completion": "search_completed", "bestmove": "7g7f", "lines": [{"move": "7g7f", "score": {"kind": "cp", "value": 20}}]},
            {"completion": "search_completed", "bestmove": "7g7f", "lines": [{"move": "7g7f", "score": {"kind": "cp", "value": 21}}]},
        ]
        self.assertIsNone(audit.stable_labels(results, ["7g7f"], 10_000))


if __name__ == "__main__":
    unittest.main()
