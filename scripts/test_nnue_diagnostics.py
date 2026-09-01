#!/usr/bin/env python3
"""Small regression tests for the NNUE diagnostic helpers.

These tests intentionally exercise only SFEN parsing and robust statistics;
they do not train, search, or launch an engine process.
"""

import importlib.util
import pathlib
import unittest


ROOT = pathlib.Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "analyze_nnue_outliers", ROOT / "analyze_nnue_outliers.py"
)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class OutlierFeatureTests(unittest.TestCase):
    def test_features_capture_observable_sfen_properties(self):
        sfen = "ln1g3nl/1r3k1g1/p2p+Np2p/5spp1/1ps6/5PP2/PPSPPS2P/2G2G1R1/LN2K3L w BPb3p 36"
        row = MODULE.features(sfen)
        self.assertEqual(row["side"], "w")
        self.assertEqual(row["hand"], "with_hand")
        self.assertEqual(row["promotion"], "with_promotion")
        self.assertEqual(row["phase"], "middle")
        self.assertEqual(row["ply"], 36)
        self.assertGreater(row["promoted_count"], 0)

    def test_features_keep_no_hand_and_even_material_distinct(self):
        sfen = "ln1g1gsnl/1r1s2k2/p1ppppbp1/6p2/1p6/2PP5/PPB1PPPPP/2R1G3L/LNS2GSNK w - 20"
        row = MODULE.features(sfen)
        self.assertEqual(row["hand"], "no_hand")
        self.assertEqual(row["promotion"], "no_promotion")
        self.assertEqual(row["material"], "black_ahead")


class OutlierStatisticTests(unittest.TestCase):
    def test_percentile_is_deterministic_and_inclusive_at_call_site(self):
        self.assertEqual(MODULE.percentile([440, 360, 335, 324, 265, 241], 0.95), 360)
        self.assertEqual(MODULE.percentile([1, 2, 3, 4], 0.0), 1)
        self.assertEqual(MODULE.percentile([1, 2, 3, 4], 1.0), 4)


if __name__ == "__main__":
    unittest.main()
