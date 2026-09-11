#!/usr/bin/env python3
import unittest
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from analyze_eval_gap import summarize


class EvalGapTest(unittest.TestCase):
    def test_reports_correlation_and_untracked_extremes(self):
        report = summarize([(-4000, -20, 3980), (0, 0, 0), (4000, 20, -3980)])
        self.assertEqual(report["positions"], 3)
        self.assertEqual(report["material_extreme_positions"], 2)
        self.assertEqual(report["extreme_untracked_positions"], 2)
        self.assertAlmostEqual(report["material_nnue_correlation"], 1.0)


if __name__ == "__main__":
    unittest.main()
