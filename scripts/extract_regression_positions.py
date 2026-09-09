#!/usr/bin/env python3
"""Extract replayable positions from candidate-loss games.

The output is JSONL suitable for a later teacher-search or label-audit pass.
It intentionally records the original search context only; it does not turn a
game result into a training target or claim that every position is causal.
"""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path


ENGINE1_COLOUR_RE = re.compile(r"^# Engine1: .* \((Black|White)\)")


def loss_games(kifu_dir: Path) -> tuple[set[int], dict[int, str]]:
    losses = set()
    colours = {}
    for path in sorted(kifu_dir.glob("game*.txt")):
        game = int(path.stem.removeprefix("game"))
        for line in path.read_text(encoding="utf-8").splitlines():
            colour = ENGINE1_COLOUR_RE.match(line)
            if colour:
                colours[game] = colour.group(1)
            if line.startswith("# Result: Engine2 Win"):
                losses.add(game)
                break
    return losses, colours


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kifu_dir", type=Path)
    parser.add_argument("transcript", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--candidate-turns-only", action="store_true")
    args = parser.parse_args()

    losses, engine1_colours = loss_games(args.kifu_dir)
    rows = []
    for line in args.transcript.read_text(encoding="utf-8").splitlines():
        record = json.loads(line)
        game = int(record["game_num"])
        if game not in losses or record.get("verdict") != "ok":
            continue
        seq = int(record["seq"])
        candidate_colour = engine1_colours.get(game)
        if candidate_colour is None:
            continue
        candidate_turn = (record["sfen_before"].split()[1] == "b") == (candidate_colour == "Black")
        if args.candidate_turns_only and not candidate_turn:
            continue
        rows.append(
            {
                "game_num": game,
                "seq": seq,
                "candidate_turn": candidate_turn,
                "sfen": record["sfen_before"],
                "legal_move_count": record["legal_move_count"],
                "raw_bestmove": record["raw_bestmove"],
                "result": "candidate_loss",
            }
        )

    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("w", encoding="utf-8") as stream:
        for row in rows:
            stream.write(json.dumps(row, ensure_ascii=False) + "\n")
    print(f"candidate-loss games: {len(losses)}")
    print(f"extracted positions: {len(rows)}")
    print(f"output: {args.output}")


if __name__ == "__main__":
    main()
