#!/usr/bin/env python3
"""Unit test for Q25e's one-factor score-blind eligibility change."""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import preregister_q25e_rule_filter_family as q25e


class Q25eFamilyTests(unittest.TestCase):
    def test_adds_only_checked_position_exclusion(self) -> None:
        prior = {
            "schema": q25e.SCHEMA,
            "status": "frozen_before_score_blind_parent_selection",
            "rule_only_eligibility": {
                "min_legal_moves": 2,
                "exclude_mate_in_one": True,
            },
            "external_teacher": {"multipv": 128},
            "fixed_training": {"objective": "pairwise"},
        }
        with tempfile.TemporaryDirectory() as directory:
            family = Path(directory) / "q25d-family.json"
            family.write_text(json.dumps(prior), encoding="utf-8")
            result = q25e.build(family)
        self.assertTrue(result["rule_only_eligibility"]["exclude_in_check"])
        self.assertEqual(result["external_teacher"]["multipv"], 128)
        self.assertEqual(result["fixed_training"]["objective"], "pairwise")


if __name__ == "__main__":
    unittest.main()
