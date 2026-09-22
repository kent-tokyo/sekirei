#!/usr/bin/env python3
"""Unit tests for the Q25d mate-aware family contract."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import preregister_q25d_mate_ordinal_family as q25d


class Q25dFamilyTests(unittest.TestCase):
    def test_changes_the_label_protocol_without_changing_model_shape(self) -> None:
        root = Path(__file__).resolve().parents[1]
        family = root / "data/runs/q25c-multipv16-label-family-20260922/family-preregistration.json"
        result = q25d.build(family)
        self.assertEqual(result["external_teacher"]["multipv"], 128)
        self.assertEqual(result["fixed_training"]["objective"], "pairwise")
        self.assertEqual(result["fixed_model"]["l1"], 256)
        self.assertEqual(result["rule_only_eligibility"]["min_legal_moves"], 2)


if __name__ == "__main__":
    unittest.main()
