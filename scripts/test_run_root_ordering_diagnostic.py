#!/usr/bin/env python3
"""Focused tests for root-order iteration trace parsing."""
from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("ordering", ROOT / "run_root_ordering_diagnostic.py")
assert SPEC and SPEC.loader
ORDERING = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ORDERING)


class RootOrderingTests(unittest.TestCase):
    def test_assigns_initial_order_rank(self) -> None:
        trace = ORDERING.parse_iteration_trace(
            "1:7g7f:12:10:exact:3:4,2:2g2f:20:30:exact:7:9", ["2g2f", "7g7f"]
        )
        self.assertEqual([entry["initial_order_rank"] for entry in trace], [2, 1])
        self.assertEqual(trace[-1]["root_mate_blunder_nodes"], 9)

    def test_rejects_non_monotonic_trace(self) -> None:
        with self.assertRaisesRegex(ValueError, "monotonic"):
            ORDERING.parse_iteration_trace("2:7g7f:12:10:exact:0:0,1:7g7f:20:30:exact:0:0", ["7g7f"])

    def test_rejects_unknown_bestmove(self) -> None:
        with self.assertRaisesRegex(ValueError, "absent"):
            ORDERING.parse_iteration_trace("1:2g2f:12:10:exact:0:0", ["7g7f"])


if __name__ == "__main__":
    unittest.main()
