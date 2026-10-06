#!/usr/bin/env python3

import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from split_gensfen_by_game import load_records, split_files


def row(sfen_board: str, side: str, result: int, ply: int, game_id: str) -> str:
    return f"{sfen_board} {side} - 1\t12\t{result}\t{ply}\t7g7f\t{game_id}\n"


class SplitGensfenByGameTest(unittest.TestCase):
    def run_split(self, text: str, *, ratio: float = 0.5, limit: int = 0):
        temporary = tempfile.TemporaryDirectory()
        root = Path(temporary.name)
        source = root / "source.txt"
        source.write_text(text, encoding="utf-8")
        train = root / "train.txt"
        validation = root / "validation.txt"
        manifest = root / "manifest.json"
        report = split_files([source], train, validation, manifest, ratio, 42, limit)
        return temporary, train.read_text(), validation.read_text(), report

    def test_whole_games_stay_in_one_arm(self):
        text = "".join(
            row(f"9/9/9/9/9/9/9/9/{index}K8", side, result, 20 + index, game_id)
            for index, (game_id, side, result) in enumerate(
                [
                    ("g1", "b", 1),
                    ("g1", "w", -1),
                    ("g2", "b", -1),
                    ("g2", "w", 1),
                    ("g3", "b", 0),
                    ("g3", "w", 0),
                    ("g4", "b", 1),
                    ("g4", "w", -1),
                ]
            )
        )
        temporary, train, validation, report = self.run_split(text)
        self.addCleanup(temporary.cleanup)
        train_games = {line.split("\t")[5] for line in train.splitlines()}
        validation_games = {line.split("\t")[5] for line in validation.splitlines()}
        self.assertFalse(train_games & validation_games)
        self.assertEqual(report["cross_arm_sfen_overlap"], 0)
        self.assertEqual(report["source"]["games"], 4)

    def test_shared_sfen_joins_games_before_split(self):
        shared = "9/9/9/9/9/9/9/9/K8"
        text = "".join(
            [
                row(shared, "b", 1, 20, "g1"),
                row(shared, "b", -1, 20, "g2"),
                row("9/9/9/9/9/9/9/9/1K7", "b", 0, 20, "g3"),
            ]
        )
        temporary, train, validation, report = self.run_split(text, ratio=0.34)
        self.addCleanup(temporary.cleanup)
        arms = {}
        for name, output in (("train", train), ("validation", validation)):
            for line in output.splitlines():
                arms[line.split("\t")[5]] = name
        self.assertEqual(arms["g1"], arms["g2"])
        self.assertEqual(report["source"]["components"], 2)

    def test_long_games_can_be_thinned_evenly(self):
        text = "".join(
            row(
                f"9/9/9/9/9/9/9/9/{index}K8",
                "b",
                -1 if index < 5 else 1,
                index * 10,
                f"g{index // 5}",
            )
            for index in range(10)
        )
        temporary, train, validation, report = self.run_split(text, limit=3)
        self.addCleanup(temporary.cleanup)
        self.assertLessEqual(len(train.splitlines()), report["train"]["games"] * 3)
        self.assertLessEqual(
            len(validation.splitlines()), report["validation"]["games"] * 3
        )

    def test_duplicate_rows_are_preserved_without_thinning(self):
        duplicate = row("9/9/9/9/9/9/9/9/K8", "b", 1, 20, "g1")
        text = duplicate + duplicate + row(
            "9/9/9/9/9/9/9/9/1K7", "b", -1, 20, "g2"
        )
        temporary, train, validation, report = self.run_split(text)
        self.addCleanup(temporary.cleanup)
        self.assertEqual(len(train.splitlines()) + len(validation.splitlines()), 3)
        self.assertEqual(report["source"]["positions"], 3)

    def test_legacy_rows_fail_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "legacy.txt"
            source.write_text("9/9/9/9/9/9/9/9/K8 b - 1\t0\t0\t20\t7g7f\n")
            with self.assertRaisesRegex(ValueError, "six-column"):
                load_records([source])


if __name__ == "__main__":
    unittest.main()
