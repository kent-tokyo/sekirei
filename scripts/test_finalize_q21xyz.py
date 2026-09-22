#!/usr/bin/env python3
"""Unit tests for Q21x/y/z cost parsing."""

from __future__ import annotations

import unittest

import finalize_q21xyz as finalizer


class Q21xyzFinalizerTests(unittest.TestCase):
    def test_median_is_stable_for_even_profile_count(self) -> None:
        self.assertEqual(finalizer.median([1, 2, 3, 4, 5, 6, 7, 8]), 4.5)


if __name__ == "__main__":
    unittest.main()
