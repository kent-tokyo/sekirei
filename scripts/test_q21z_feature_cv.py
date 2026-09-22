#!/usr/bin/env python3
"""Import and contract smoke tests for Q21z feature CV."""

from __future__ import annotations

import unittest

import run_q21z_feature_cv as q21z


class Q21zFeatureCvTests(unittest.TestCase):
    def test_require_fails_closed(self) -> None:
        with self.assertRaisesRegex(ValueError, "closed"):
            q21z.require(False, "closed")


if __name__ == "__main__":
    unittest.main()
