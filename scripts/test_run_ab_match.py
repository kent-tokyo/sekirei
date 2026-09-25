#!/usr/bin/env python3
"""Unit tests for the SPRT helpers in run_ab_match.py (stdlib unittest only).

Run: python3 scripts/test_run_ab_match.py
"""

import math
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from run_ab_match import sprt_bounds, sprt_llr  # noqa: E402


class SprtTest(unittest.TestCase):
    def test_bounds_match_wald_for_five_percent_errors(self):
        lower, upper = sprt_bounds()
        self.assertAlmostEqual(lower, math.log(0.05 / 0.95))
        self.assertAlmostEqual(upper, math.log(0.95 / 0.05))

    def test_llr_sign_follows_the_score(self):
        self.assertEqual(sprt_llr(0, 0, 0, 0, 10), 0.0)
        self.assertGreater(sprt_llr(60, 40, 20, 0, 10), 0)
        self.assertLess(sprt_llr(40, 60, 20, 0, 10), 0)

    def test_strong_results_cross_the_bounds(self):
        lower, upper = sprt_bounds()
        # About +70 Elo over 600 games clearly accepts H1 for [0, 10].
        self.assertGreater(sprt_llr(300, 180, 120, 0, 10), upper)
        # About -70 Elo accepts H0.
        self.assertLess(sprt_llr(180, 300, 120, 0, 10), lower)
        # An even result stays between the bounds after 200 games.
        self.assertTrue(lower < sprt_llr(80, 80, 40, 0, 10) < upper)

    def test_llr_scales_with_games_at_a_fixed_score(self):
        small = sprt_llr(60, 40, 20, 0, 10)
        large = sprt_llr(600, 400, 200, 0, 10)
        self.assertAlmostEqual(large, 10 * small)


if __name__ == "__main__":
    unittest.main()
