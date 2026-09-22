#!/usr/bin/env python3
"""Unit tests for Q21x's independent baseline helpers."""

from __future__ import annotations

import unittest

import prepare_q21x_baseline as prepare


class Q21xBaselineTests(unittest.TestCase):
    def test_sources_and_groups_must_be_unique(self) -> None:
        rows = [
            {"source": {"source_key": "a", "derived_group": "ga"}},
            {"source": {"source_key": "b", "derived_group": "gb"}},
        ]
        self.assertEqual(prepare.source_keys(rows), {"a", "b"})
        self.assertEqual(prepare.identities(rows), {"ga", "gb"})

    def test_duplicate_source_fails_closed(self) -> None:
        rows = [
            {"source": {"source_key": "a", "derived_group": "ga"}},
            {"source": {"source_key": "a", "derived_group": "gb"}},
        ]
        with self.assertRaisesRegex(ValueError, "sources"):
            prepare.source_keys(rows)


if __name__ == "__main__":
    unittest.main()
