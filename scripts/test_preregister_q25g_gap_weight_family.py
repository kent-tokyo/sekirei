#!/usr/bin/env python3
"""Tests for Q25g's single-factor registration."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import preregister_q25g_gap_weight_family as q25g


class Q25gGapWeightFamilyTests(unittest.TestCase):
    def test_adds_only_capped_gap_weighting(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "q25e.json"
            path.write_text(
                """{
                  "schema": "sekirei.q25a-external-label-family-preregistration.v1",
                  "status": "frozen_before_score_blind_parent_selection",
                  "family_id": "q25e-example",
                  "fixed_training": {"objective": "pairwise", "epochs": 60},
                  "rule_only_eligibility": {"exclude_in_check": true}
                }""",
                encoding="utf-8",
            )
            result = q25g.build(path)
        self.assertEqual(result["fixed_training"]["pair_weighting"], "capped-teacher-gap")
        self.assertEqual(result["fixed_training"]["epochs"], 60)
        self.assertEqual(result["rule_only_eligibility"], {"exclude_in_check": True})
        self.assertEqual(result["single_factor"], "pairwise update weighting: capped external teacher centipawn gap")


if __name__ == "__main__":
    unittest.main()
