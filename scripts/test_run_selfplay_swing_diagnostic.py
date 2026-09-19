#!/usr/bin/env python3
"""Focused tests for history-preserving self-play swing selection."""

from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("swing", ROOT / "run_selfplay_swing_diagnostic.py")
assert SPEC and SPEC.loader
SWING = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SWING)


class SwingDiagnosticTests(unittest.TestCase):
    def test_case_preserves_game_local_history(self) -> None:
        rows = [
            {"game_num": 1, "seq": 10, "verdict": "ok", "sfen_before": "start", "raw_bestmove": "7g7f", "search": {}},
            {"game_num": 1, "seq": 11, "verdict": "ok", "sfen_before": "after", "raw_bestmove": "3c3d", "search": {}},
        ]
        games = {1: rows}
        item = SWING.case(games, 1, 11, "target")
        self.assertEqual(item["initial_sfen"], "start")
        self.assertEqual(item["history_before_usi"], ["7g7f"])
        self.assertEqual(item["expected_sfen"], "after")

    def test_parse_selection_rejects_duplicate_labels(self) -> None:
        with self.assertRaises(ValueError):
            SWING.parse_selection(["1:2:a", "3:4:a"])

    def test_legacy_unweighted_source_is_not_nnue_score_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            transcript = root / "transcript.jsonl"
            transcript.write_text("", encoding="utf-8")
            (root / "run-manifest.json").write_text(
                json.dumps({"weights": {"path": None, "sha256": None}}), encoding="utf-8"
            )
            source = SWING.source_evaluator(transcript)
        self.assertEqual(source["kind"], "material_or_unknown_legacy")
        self.assertFalse(source["observed_scores_usable"])


if __name__ == "__main__":
    unittest.main()
