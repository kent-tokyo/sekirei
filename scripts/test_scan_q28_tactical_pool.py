#!/usr/bin/env python3
"""Focused no-engine tests for Q28 tactical-pool scan contracts."""

from __future__ import annotations

import importlib.util
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("q28_scan", ROOT / "scan_q28_tactical_pool.py")
assert SPEC and SPEC.loader
Q28_SCAN = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(Q28_SCAN)


class Q28TacticalPoolTests(unittest.TestCase):
    def test_requires_positive_scan_cap(self) -> None:
        with self.assertRaises(ValueError):
            Q28_SCAN.require(False, "source scan cap must be positive")

    def test_schema_is_specific_to_score_free_pool_audit(self) -> None:
        self.assertEqual(Q28_SCAN.SCHEMA, "sekirei.q28-tactical-pool-scan.v1")
        self.assertNotIn("teacher", Q28_SCAN.SCHEMA)


if __name__ == "__main__":
    unittest.main()
