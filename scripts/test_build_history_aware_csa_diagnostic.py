#!/usr/bin/env python3
"""Regression tests for history-aware CSA diagnostic corpus generation."""
from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("builder", ROOT / "scripts/build_history_aware_csa_diagnostic.py")
assert SPEC and SPEC.loader
builder = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(builder)


class HistoryAwareCorpusTests(unittest.TestCase):
    def test_preserves_initial_sfen_and_usi_prefix(self):
        with tempfile.TemporaryDirectory() as temp:
            replay_dir = Path(temp)
            replay = {
                "schema": "sekirei.csa-replay.v2",
                "initial_sfen": "4k4/9/9/9/9/9/9/9/4K4 b - 1",
                "positions": [
                    {
                        "ply": 0,
                        "pre_move_sfen": "4k4/9/9/9/9/9/9/9/4K4 b - 1",
                        "history_before_usi": [],
                        "captured_piece": None,
                        "side_to_move_in_check": False,
                    },
                    {
                        "ply": 1,
                        "pre_move_sfen": "4k4/9/9/9/9/9/9/4K4 w - 2",
                        "history_before_usi": ["5i5h"],
                        "captured_piece": "Fu",
                        "side_to_move_in_check": False,
                    },
                ],
            }
            path = replay_dir / "game0001.json"
            path.write_text(json.dumps(replay), encoding="utf-8")
            document = builder.build(replay_dir, 2)

        self.assertEqual(document["schema"], builder.SCHEMA)
        self.assertEqual(document["invalid_replays"], [])
        self.assertEqual(len(document["positions"]), 2)
        self.assertEqual(len(document["entries"]), 2)
        second = document["positions"][1]
        self.assertEqual(second["initial_sfen"], replay["initial_sfen"])
        self.assertEqual(second["history_before_usi"], ["5i5h"])
        self.assertEqual(second["category"], "capture_position")
        self.assertEqual(
            document["entries"][1]["position"]["history_before_usi"],
            ["5i5h"],
        )
        self.assertEqual(document["entries"][1]["source"]["category"], "capture_position")

    def test_bad_history_is_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            replay_dir = Path(temp)
            (replay_dir / "bad.json").write_text(json.dumps({
                "schema": "sekirei.csa-replay.v2",
                "initial_sfen": "x",
                "positions": [{"ply": 0, "pre_move_sfen": "x", "history_before_usi": [""]}],
            }), encoding="utf-8")
            document = builder.build(replay_dir, 1)
        self.assertEqual(document["positions"], [])
        self.assertEqual(document["invalid_replays"][0]["error"], "invalid_usi_history_at_ply_0")

    def test_prioritizes_positions_after_drop_and_promoted_piece_move(self):
        replay = {
            "schema": "sekirei.csa-replay.v2",
            "initial_sfen": "x",
            "positions": [
                {"ply": 0, "pre_move_sfen": "x", "history_before_usi": []},
                {
                    "ply": 1,
                    "pre_move_sfen": "y",
                    "history_before": ["+0055HI"],
                    "history_before_usi": ["R*5e"],
                },
                {
                    "ply": 2,
                    "pre_move_sfen": "z",
                    "history_before": ["+8822UM"],
                    "history_before_usi": ["8h2b+"],
                },
            ],
        }
        with tempfile.TemporaryDirectory() as temp:
            replay_dir = Path(temp)
            (replay_dir / "game.json").write_text(json.dumps(replay), encoding="utf-8")
            document = builder.build(replay_dir, 3)
        self.assertEqual(
            [entry["category"] for entry in document["positions"]],
            ["opening_control", "after_drop", "after_promoted_piece_move"],
        )

    def test_max_games_uses_deterministic_prefix(self):
        with tempfile.TemporaryDirectory() as temp:
            replay_dir = Path(temp)
            for name in ("game0002.json", "game0001.json"):
                (replay_dir / name).write_text(json.dumps({
                    "schema": "sekirei.csa-replay.v2",
                    "initial_sfen": "x",
                    "positions": [{"ply": 0, "pre_move_sfen": "x", "history_before_usi": []}],
                }), encoding="utf-8")
            document = builder.build(replay_dir, 1, max_games=1)
        self.assertEqual([entry["id"] for entry in document["positions"]], ["game0001-ply000"])

    def test_excludes_cross_game_duplicate_initial_history(self):
        with tempfile.TemporaryDirectory() as temp:
            replay_dir = Path(temp)
            for name in ("game0001.json", "game0002.json"):
                (replay_dir / name).write_text(json.dumps({
                    "schema": "sekirei.csa-replay.v2", "initial_sfen": "start",
                    "positions": [{"ply": 0, "pre_move_sfen": "start", "history_before_usi": []}],
                }), encoding="utf-8")
            document = builder.build(replay_dir, 1)
        self.assertEqual([entry["id"] for entry in document["positions"]], ["game0001-ply000"])
        self.assertEqual(document["duplicate_history_excluded"][0]["id"], "game0002-ply000")


if __name__ == "__main__":
    unittest.main()
