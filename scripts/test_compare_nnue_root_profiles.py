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
        self.assertEqual(value["hand_pieces"], 3)
        self.assertEqual(value["black_hand_material_cp"], 200)
        self.assertEqual(value["white_hand_material_cp"], 100)
        self.assertEqual(value["material_imbalance_cp"], 1400)
        self.assertEqual(value["material_stm_cp"], 1400)
        self.assertEqual(value["material_band"], "stm_ahead")

    def test_attributes_flip_hand_material_for_white_to_move(self) -> None:
        value = PROFILES.attributes("4k4/9/9/9/9/9/9/9/4K4 w R2p 1")
        self.assertEqual(value["material_imbalance_cp"], 840)
        self.assertEqual(value["material_stm_cp"], -840)
        self.assertEqual(value["material_band"], "stm_behind")


if __name__ == "__main__":
    unittest.main()
