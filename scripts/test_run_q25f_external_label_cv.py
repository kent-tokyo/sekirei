#!/usr/bin/env python3
"""Unit tests for Q25f's score-blind parent split helpers."""

from __future__ import annotations

import unittest

import run_q25f_external_label_cv as q25f


class Q25fExternalLabelCvTests(unittest.TestCase):
    def test_each_stratum_contributes_one_parent_per_fold(self) -> None:
        reserve = {
            "positions": [
                {"id": f"{category}-{index}", "category": category}
                for category in ("opening/balanced", "endgame/stm_behind")
                for index in range(8)
            ]
        }
        result = q25f.folds(reserve)
        self.assertEqual(len(result), 8)
        self.assertTrue(all(len(fold) == 2 for fold in result))
        self.assertEqual(set().union(*result), {row["id"] for row in reserve["positions"]})

    def test_split_requires_eight_parents_per_stratum(self) -> None:
        with self.assertRaisesRegex(ValueError, "exactly eight"):
            q25f.folds({"positions": [{"id": "a", "category": "opening/balanced"}]})

    def test_pair_subset_preserves_only_contract_fields(self) -> None:
        document = {
            "schema": "sekirei.root-rank-pairs.v1",
            "diagnostic_only": True,
            "strength_claim": "not_permitted",
            "source_contract": {},
            "source_teacher": {},
            "pair_selection": "adjacent",
            "unexpected": "not copied",
            "pairs": [{"parent_id": "a"}, {"parent_id": "b"}],
        }
        result = q25f.pair_subset(document, {"a"})
        self.assertNotIn("unexpected", result)
        self.assertEqual(result["pairs"], [{"parent_id": "a"}])


if __name__ == "__main__":
    unittest.main()
