#!/usr/bin/env python3
"""Focused selection tests for the Q28 score-free boundary."""

from __future__ import annotations

import importlib.util
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("q28_prepare", ROOT / "prepare_q28_tactical_source_family.py")
assert SPEC and SPEC.loader
Q28 = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(Q28)


def row(index: int, tactical_class: str) -> dict[str, object]:
    return {
        "category": "opening/balanced",
        "sfen": f"lnsgkgsnl/1r5b1/p1ppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b {index + 1}P 1",
        "tactical_class": tactical_class,
        "rule_facts": {},
        "source": {"source_key": f"source-{index:02d}"},
    }


class Q28TacticalSelectionTests(unittest.TestCase):
    def test_reserves_two_nonchecking_train_parents_and_unique_sources(self) -> None:
        rows = [row(index, "checking_resource") for index in range(8)]
        rows += [row(8, "capture_resource"), row(9, "quiet")]
        train, holdout = Q28.select_group(
            rows,
            category="opening/balanced",
            seed=1,
            minimum_nonchecking=2,
            used_sources=set(),
            used_identities=set(),
        )
        self.assertEqual(len(train), 8)
        self.assertEqual(len(holdout), 2)
        self.assertGreaterEqual(sum(item["tactical_class"] != "checking_resource" for item in train), 2)
        all_sources = [item["source"]["source_key"] for item in train + holdout]
        self.assertEqual(len(all_sources), len(set(all_sources)))

    def test_fails_closed_when_nonchecking_quota_is_unavailable(self) -> None:
        with self.assertRaises(ValueError):
            Q28.select_group(
                [row(index, "checking_resource") for index in range(10)],
                category="opening/balanced",
                seed=1,
                minimum_nonchecking=2,
                used_sources=set(),
                used_identities=set(),
            )


if __name__ == "__main__":
    unittest.main()
