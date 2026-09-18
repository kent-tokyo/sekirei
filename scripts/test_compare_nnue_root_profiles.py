#!/usr/bin/env python3
"""Focused tests for root-profile SFEN classification."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("profiles", ROOT / "compare_nnue_root_profiles.py")
assert SPEC and SPEC.loader
PROFILES = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PROFILES)


class RootProfileTests(unittest.TestCase):
    def test_attributes_count_material_hands_and_promotions(self) -> None:
        value = PROFILES.attributes("4k4/9/9/9/4+R4/9/9/9/4K4 b 2Pp 21")
        self.assertEqual(value["phase"], "middlegame")
        self.assertEqual(value["promoted_pieces"], 1)
        self.assertEqual(value["hand_piece_kinds"], 2)
        self.assertEqual(value["material_imbalance"], 12)
        self.assertEqual(value["material_band"], "black_ahead")


if __name__ == "__main__":
    unittest.main()
