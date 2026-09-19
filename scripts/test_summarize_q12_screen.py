#!/usr/bin/env python3
"""Regression tests for Q12 aggregate-manifest construction."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from summarize_q12_screen import summarize


def write_pair(root: Path, number: int, wins: int, draws: int, losses: int) -> None:
    pair = root / f"pair{number}"
    (pair / "csa").mkdir(parents=True)
    result = {
        "status": "complete", "games": 2,
        "engine1_command": "sekirei", "engine1_args": "candidate.bin",
        "engine2_command": "sekirei", "engine2_args": "teacher.bin",
        "engine1_options": {"Threads": "1", "SearchMode": "Speculative", "SpecTopN": "0", "MultiPV": "1", "UseBook": "false", "NnueOutput": "residual-material"},
        "engine1_wins": wins, "draws": draws, "engine2_wins": losses,
        "invalid_games": [], "artifact_write_failures": [],
    }
    csa = {"status": "complete", "games_completed": 2, "byoyomi_ms": 10, "max_moves": 160}
    (pair / "result.json").write_text(json.dumps(result))
    (pair / "csa" / "manifest.json").write_text(json.dumps(csa))


class Q12SummaryTests(unittest.TestCase):
    def test_collects_each_pair_once_and_rejects_sub_half_score(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_pair(root, 2, 0, 0, 2)
            write_pair(root, 1, 1, 0, 1)
            manifest = summarize(root)
        self.assertEqual(manifest["verdict"], "REJECTED_SCREEN")
        self.assertEqual(manifest["results"]["candidate_score"], 0.25)
        self.assertEqual(manifest["inputs"]["runs"], [str(root / "pair1" / "result.json"), str(root / "pair2" / "result.json")])

    def test_rejects_incomplete_pair(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            write_pair(root, 1, 1, 0, 1)
            result = json.loads((root / "pair1" / "result.json").read_text())
            result["status"] = "incomplete"
            (root / "pair1" / "result.json").write_text(json.dumps(result))
            with self.assertRaisesRegex(ValueError, "not complete"):
                summarize(root)


if __name__ == "__main__":
    unittest.main()
