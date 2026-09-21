#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
import json
import tempfile
from pathlib import Path


SCRIPT = Path(__file__).with_name("prepare_q21j_failure_audit.py")
SPEC = importlib.util.spec_from_file_location("prepare_q21j", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


def row(game: int, seq: int, pid: int, score: int, move: str) -> dict:
    return {
        "game_num": game, "seq": seq, "pid": pid, "raw_bestmove": move,
        "sfen_before": f"sfen-{game}-{seq}", "search": {"score_cp": score, "score_mate": None},
    }


def main() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        kifu = root / "kifu"
        kifu.mkdir()
        transcript = root / "transcript.jsonl"
        results = root / "results.jsonl"
        rows = []
        outcomes = []
        for game in range(1, 33):
            # Engine1 pid 10 is Black in game 1 and remains the candidate.
            game_rows = [
                row(game, 0, 10 if game % 2 else 20, 100, "7g7f"),
                row(game, 1, 20 if game % 2 else 10, 50, "3c3d"),
                row(game, 2, 10 if game % 2 else 20, -250 if game == 1 else 80, "2g2f"),
                row(game, 3, 20 if game % 2 else 10, 40, "8c8d"),
            ]
            rows.extend(game_rows)
            outcomes.append({"id": f"pos{game}", "result": "candidate_win" if game > 25 else "baseline_win"})
            (kifu / f"game{game:04d}.txt").write_text(
                ("# Engine1: Sekirei (Black)\n" if game % 2 else "# Engine1: Sekirei (White)\n")
                + "position sfen 9/9/9/9/9/9/9/9/9 b - 1 moves 7g7f 3c3d 2g2f 8c8d\n",
                encoding="utf-8",
            )
        transcript.write_text("\n".join(json.dumps(item) for item in rows) + "\n", encoding="utf-8")
        results.write_text("\n".join(json.dumps(item) for item in outcomes) + "\n", encoding="utf-8")
        document = MODULE.prepare(transcript, results, kifu)
        assert document["candidate_pid"] == 10
        assert document["counts"] == {"games": 32, "candidate_wins": 7, "candidate_losses": 25, "first_large_drop": 1}
        assert document["positions"][0]["selection"]["rule"] == "first_candidate_score_drop_ge_300cp"
        assert document["positions"][0]["position"]["history_before_usi"] == []
        assert document["positions"][1]["position"]["history_before_usi"] == ["7g7f"]


if __name__ == "__main__":
    main()
