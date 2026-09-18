#!/usr/bin/env python3
"""Focused tests for deterministic NNUE pilot corpus freezing."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("pilot", ROOT / "freeze_learning_pilot_corpus.py")
assert SPEC and SPEC.loader
PILOT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PILOT)


def row(sfen: str, source: str) -> dict:
    return {"sfen": sfen, "source": {"path": source}, "tags": {"phase": "opening"}}


class FreezeLearningPilotCorpusTests(unittest.TestCase):
    def test_canonical_sfen_drops_move_number(self) -> None:
        self.assertEqual(PILOT.canonical_sfen("board b P 1"), PILOT.canonical_sfen("board b P 99"))

    def test_selection_is_per_source_deterministic_and_exclusion_aware(self) -> None:
        rows = [
            row("a b - 1", "a"), row("b b - 1", "a"), row("c b - 1", "a"),
            row("d b - 1", "b"), row("e b - 1", "b"), row("a b - 99", "c"),
        ]
        first, rejected = PILOT.freeze(rows, {"b b -"}, per_source=1, seed=42)
        second, _ = PILOT.freeze(list(reversed(rows)), {"b b -"}, per_source=1, seed=42)
        self.assertEqual({item["sfen"] for item in first}, {item["sfen"] for item in second})
        self.assertEqual(rejected, {"excluded": 1, "duplicate": 1})
        self.assertEqual(len(first), 2)


if __name__ == "__main__":
    unittest.main()
