#!/usr/bin/env python3
"""Regression tests for the Q26 fixed-budget command contract."""

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from run_q26_search_profile import build_command  # noqa: E402


class SearchProfileCommandTest(unittest.TestCase):
    def test_fixed_nodes_does_not_enable_fixed_depth_mode(self):
        command = build_command(
            Path("search"),
            {"sfen": "position"},
            100_000,
            None,
            50,
            None,
            True,
        )

        self.assertIn("--nodes", command)
        self.assertNotIn("--max-depth", command)
        self.assertIn("--profile-cost", command)
        self.assertNotIn("--weights", command)

    def test_fixed_time_keeps_the_requested_depth_cap_and_weights(self):
        command = build_command(
            Path("search"),
            {"sfen": "position"},
            100_000,
            250,
            32,
            Path("weights.bin"),
            False,
        )

        self.assertIn("--time-ms", command)
        self.assertIn("--max-depth", command)
        self.assertIn("32", command)
        self.assertIn("--weights", command)
        self.assertNotIn("--profile-cost", command)


if __name__ == "__main__":
    unittest.main()
