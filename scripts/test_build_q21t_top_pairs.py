#!/usr/bin/env python3
"""Regression tests for direct top-vs-rest pair construction."""

from __future__ import annotations

import unittest

import build_q21t_top_pairs as q21t


class Q21tPairBuilderTests(unittest.TestCase):
    def test_all_tied_scores_have_no_strict_lower_move(self) -> None:
        ranked, top, lower = q21t.ranked_top_and_lower({"7g7f": 0, "2g2f": 0}, 8)
        self.assertEqual(len(ranked), 2)
        self.assertEqual(set(top), {"7g7f", "2g2f"})
        self.assertEqual(lower, [])

    def test_strict_scores_keep_every_tied_top_against_lower_moves(self) -> None:
        _, top, lower = q21t.ranked_top_and_lower(
            {"7g7f": 20, "2g2f": 20, "5g5f": 0}, 8
        )
        self.assertEqual(set(top), {"7g7f", "2g2f"})
        self.assertEqual(lower, [("5g5f", 0)])


if __name__ == "__main__":
    unittest.main()
