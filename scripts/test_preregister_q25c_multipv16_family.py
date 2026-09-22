#!/usr/bin/env python3
"""Pure-data regression tests for Q25c's one-factor family contract."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

import preregister_q25c_multipv16_family as q25c


class Q25cFamilyTests(unittest.TestCase):
    def test_only_multipv_changes_in_the_teacher_contract(self) -> None:
        prior = {
            "schema": q25c.SCHEMA,
            "status": "frozen_before_score_blind_parent_selection",
            "family_id": "q25a",
            "fixed_model": {"l1": 256, "l2": 32},
            "fixed_training": {"seed": 42},
            "screen": {"same_time": "fixed"},
            "external_teacher": {
                "multipv": 4,
                "max_depth": 10,
                "threads": 1,
                "hash_mb": 256,
                "usi_options": {"Threads": "1", "MultiPV": "4", "USI_Hash": "256"},
            },
            "new_data_boundary": {"train_parents": 72, "holdout_parents": 18},
            "prohibitions": [],
            "inputs": {"old": "not copied"},
        }
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "q25a.json"
            path.write_text(json.dumps(prior), encoding="utf-8")
            result = q25c.build(path)

        self.assertEqual(result["family_id"], "q25c-yaneuraou-v900-suisho5-multipv16-labels-v1")
        self.assertEqual(result["external_teacher"]["multipv"], 16)
        self.assertEqual(result["external_teacher"]["usi_options"]["MultiPV"], "16")
        self.assertEqual(result["external_teacher"]["usi_options"]["Threads"], "1")
        self.assertEqual(result["fixed_model"], prior["fixed_model"])
        self.assertEqual(result["fixed_training"], prior["fixed_training"])
        self.assertEqual(result["screen"], prior["screen"])
        self.assertEqual(result["new_data_boundary"], prior["new_data_boundary"])


if __name__ == "__main__":
    unittest.main()
