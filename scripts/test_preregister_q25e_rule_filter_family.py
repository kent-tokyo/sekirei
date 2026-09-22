#!/usr/bin/env python3
"""Unit test for Q25e's one-factor score-blind eligibility change."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import preregister_q25e_rule_filter_family as q25e


class Q25eFamilyTests(unittest.TestCase):
    def test_adds_only_checked_position_exclusion(self) -> None:
        root = Path(__file__).resolve().parents[1]
        family = root / "data/runs/q25d-mate-ordinal-label-family-20260922/family-preregistration.json"
        result = q25e.build(family)
        self.assertTrue(result["rule_only_eligibility"]["exclude_in_check"])
        self.assertEqual(result["external_teacher"]["multipv"], 128)
        self.assertEqual(result["fixed_training"]["objective"], "pairwise")


if __name__ == "__main__":
    unittest.main()
