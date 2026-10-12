#!/usr/bin/env python3
"""Unit tests for the Q25d mate-aware family contract."""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import preregister_q25d_mate_ordinal_family as q25d


class Q25dFamilyTests(unittest.TestCase):
    def test_changes_the_label_protocol_without_changing_model_shape(self) -> None:
        prior = {
            "schema": q25d.SCHEMA,
            "status": "frozen_before_score_blind_parent_selection",
            "external_teacher": {
                "multipv": 16,
                "usi_options": {"MultiPV": "16"},
            },
            "fixed_training": {"objective": "listwise", "temperature_cp": 100},
            "fixed_model": {"l1": 256},
        }
        with tempfile.TemporaryDirectory() as directory:
            family = Path(directory) / "q25c-family.json"
            family.write_text(json.dumps(prior), encoding="utf-8")
            result = q25d.build(family)
        self.assertEqual(result["external_teacher"]["multipv"], 128)
        self.assertEqual(result["fixed_training"]["objective"], "pairwise")
        self.assertEqual(result["fixed_model"]["l1"], 256)
        self.assertEqual(result["rule_only_eligibility"]["min_legal_moves"], 2)


if __name__ == "__main__":
    unittest.main()
