#!/usr/bin/env python3
"""Tests for Q21w independent-parent inventory and allocation."""

from __future__ import annotations

import unittest

import prepare_q21w_coverage as q21w


def row(category: str, source: str, identity: str) -> dict:
    return {
        "category": category,
        "sfen": f"{identity} b - 1",
        "symmetry_identity": identity,
        "source": {
            "source_key": source,
            "sha256": source.removeprefix("source-"),
            "derived_group": identity,
        },
    }


class Q21wCoverageTests(unittest.TestCase):
    def test_extracts_csa_paths_recursively(self) -> None:
        values = list(
            q21w.strings(
                {"a": [{"path": "data/csa/2025/a.csa"}], "b": "not-csa.json"}
            )
        )
        self.assertIn("data/csa/2025/a.csa", values)
        self.assertIn("not-csa.json", values)

    def test_allocate_keeps_train_holdout_sources_and_groups_disjoint(self) -> None:
        candidates = {}
        counter = 0
        for category in q21w.STRATA:
            values = []
            for _ in range(4):
                counter += 1
                values.append(row(category, f"source-{counter:04d}", f"identity-{counter:04d}"))
            candidates[category] = values
        train, holdout = q21w.allocate(candidates, 2, 1, 7)
        self.assertEqual(len(train), 18)
        self.assertEqual(len(holdout), 9)
        train_sources = {item["source"]["source_key"] for item in train}
        holdout_sources = {item["source"]["source_key"] for item in holdout}
        train_groups = {item["source"]["derived_group"] for item in train}
        holdout_groups = {item["source"]["derived_group"] for item in holdout}
        self.assertFalse(train_sources & holdout_sources)
        self.assertFalse(train_groups & holdout_groups)

    def test_allocate_fails_closed_when_one_stratum_is_short(self) -> None:
        candidates = {}
        counter = 0
        for category in q21w.STRATA:
            count = 2 if category == "opening/stm_ahead" else 3
            values = []
            for _ in range(count):
                counter += 1
                values.append(row(category, f"source-{counter:04d}", f"identity-{counter:04d}"))
            candidates[category] = values
        with self.assertRaisesRegex(ValueError, "opening/stm_ahead"):
            q21w.allocate(candidates, 2, 1, 7)


if __name__ == "__main__":
    unittest.main()
