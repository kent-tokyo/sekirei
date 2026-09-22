#!/usr/bin/env python3
"""Unit tests for Q21y's stratified capacity-only CV helpers."""

from __future__ import annotations

import unittest

import run_q21y_capacity_cv as q21y


class Q21yCapacityCvTests(unittest.TestCase):
    def test_each_stratum_contributes_one_parent_per_fold(self) -> None:
        corpus = {
            "positions": [
                {"id": f"{category}-{index}", "category": category}
                for category in ("opening/balanced", "endgame/stm_behind")
                for index in range(8)
            ]
        }
        folds = q21y.folds(corpus)
        self.assertEqual(len(folds), 8)
        self.assertTrue(all(len(fold) == 2 for fold in folds))
        self.assertEqual(len(set().union(*folds)), 16)

    def test_pair_subset_does_not_add_unknown_schema_fields(self) -> None:
        document = {
            "schema": "sekirei.root-rank-pairs.v1",
            "diagnostic_only": True,
            "strength_claim": "not_permitted",
            "source_contract": {},
            "source_teacher": {},
            "pair_selection": "top-vs-rest",
            "ignored": "not copied",
            "pairs": [{"parent_id": "a"}, {"parent_id": "b"}],
        }
        subset = q21y.pair_subset(document, {"a"})
        self.assertNotIn("ignored", subset)
        self.assertEqual(subset["pairs"], [{"parent_id": "a"}])


if __name__ == "__main__":
    unittest.main()
