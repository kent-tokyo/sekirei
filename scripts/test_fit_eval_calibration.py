#!/usr/bin/env python3
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from fit_eval_calibration import fit


class EvalCalibrationTest(unittest.TestCase):
    def test_affine_fit_improves_matching_samples(self):
        report = fit([(-2.0, -5.0), (0.0, 1.0), (2.0, 7.0)])
        self.assertEqual(report["positions"], 3)
        self.assertTrue(report["usable_for_engine"])
        self.assertAlmostEqual(report["slope"], 3.0)

    def test_constant_signal_is_not_usable(self):
        report = fit([(0.0, -5.0), (0.0, 1.0), (0.0, 7.0)])
        self.assertFalse(report["usable_for_engine"])


if __name__ == "__main__":
    unittest.main()
