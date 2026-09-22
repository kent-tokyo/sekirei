#!/usr/bin/env python3
"""Tests for Q25h's mixed terminal ordinal registration."""

from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

import preregister_q25h_terminal_ordinal_family as q25h


class Q25hTerminalOrdinalFamilyTests(unittest.TestCase):
    def test_preserves_recipe_and_adds_only_label_semantics(self) -> None:
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
            result = q25h.build(path)
        self.assertTrue(result["pair_semantics"]["allow_mixed_terminal_ordinal"])
        self.assertEqual(result["fixed_training"], {"objective": "pairwise", "epochs": 60})
        self.assertEqual(result["rule_only_eligibility"], {"exclude_in_check": True})


if __name__ == "__main__":
    unittest.main()
