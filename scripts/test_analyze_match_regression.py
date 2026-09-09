#!/usr/bin/env python3
"""Regression tests for multi-directory match aggregation."""

import tempfile
from pathlib import Path

from analyze_match_regression import aggregate, parse_game


def game(result: str, colour: str = "Black") -> str:
    return "\n".join(
        [
            f"# Engine1: candidate ({colour})",
            f"# Result: {result}",
            "position sfen 9/9/9/9/9/9/9/9/4K4 b - 1 moves 7g7f",
        ]
    )


def test_parse_game_and_aggregate() -> None:
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "game0001.txt"
        path.write_text(game("Engine1 Win"), encoding="utf-8")
        row = parse_game(path)
        assert row is not None
        row["length_bucket"] = "short"
        report = aggregate([row])
        assert report["games"] == 1
        assert report["candidate_score"] == 1.0
        assert report["result_counts"] == {"Engine1 Win": 1}


if __name__ == "__main__":
    test_parse_game_and_aggregate()
    print("match regression tests: ok")
