#!/usr/bin/env python3
"""Small pure-data checks for Q25a's coverage decision boundary."""

from __future__ import annotations

import unittest
from collections import Counter


def covered(rows: list[dict[str, object]], categories: list[str], per_category: int) -> bool:
    usable = [row for row in rows if row["ordinary_labeled_move_count"] >= 2]
    counts = Counter(row["category"] for row in usable)
    return len(usable) == len(rows) and all(counts[category] == per_category for category in categories)


class CoverageTests(unittest.TestCase):
    def test_full_coverage_passes(self) -> None:
        categories = ["a", "b"]
        rows = [{"category": category, "ordinary_labeled_move_count": 2} for category in categories for _ in range(2)]
        self.assertTrue(covered(rows, categories, 2))

    def test_missing_scoreable_move_fails(self) -> None:
        categories = ["a", "b"]
        rows = [{"category": category, "ordinary_labeled_move_count": 2} for category in categories for _ in range(2)]
        rows[-1]["ordinary_labeled_move_count"] = 1
        self.assertFalse(covered(rows, categories, 2))


if __name__ == "__main__":
    unittest.main()
