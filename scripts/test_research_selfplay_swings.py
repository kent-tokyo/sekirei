#!/usr/bin/env python3
"""Unit tests for frozen self-play swing selection."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("swings", ROOT / "research_selfplay_swings.py")
assert SPEC and SPEC.loader
SWINGS = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SWINGS)


class SwingSelectionTests(unittest.TestCase):
    def test_duplicate_position_and_history_is_selected_once(self) -> None:
        base = {"sfen": "9/9/9/9/9/9/9/9/9 b - 1", "initial_sfen": "9/9/9/9/9/9/9/9/9 b - 1", "history": [], "actual_move": "7g7f"}
        rows = [dict(base, game=1), dict(base, game=2), dict(base, sfen="9/9/9/9/9/9/9/9/9 w - 2", game=3)]
        selected = SWINGS.unique_rows({"top_swings": rows}, 2)
        self.assertEqual([row["game"] for row in selected], [1, 3])


if __name__ == "__main__":
    unittest.main()
