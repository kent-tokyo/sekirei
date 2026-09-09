#!/usr/bin/env python3
"""Classify the extracted candidate-loss position corpus.

The output is a compact sampling plan for teacher-label audits. It is not a
training set and does not infer causality from a game result.
"""

from __future__ import annotations

import argparse
import json
from collections import Counter, defaultdict
from pathlib import Path


def classify(row: dict[str, object]) -> dict[str, object]:
    fields = str(row["sfen"]).split()
    board, side, hand, ply_text = fields[:4]
    pieces = [char for char in board if char.isalpha()]
    return {
        **row,
        "side_to_move": side,
        "hand": "with_hand" if hand != "-" else "no_hand",
        "promotion": "with_promotion" if "+" in board else "no_promotion",
        "phase": "early" if int(ply_text) < 20 else "middle" if int(ply_text) < 40 else "late",
        "piece_count": len(pieces),
        "ply": int(ply_text),
        "legal_move_band": (
            "forced" if int(row["legal_move_count"]) <= 10 else
            "narrow" if int(row["legal_move_count"]) <= 40 else
            "normal" if int(row["legal_move_count"]) <= 120 else
            "wide"
        ),
    }


def summarize(rows: list[dict[str, object]]) -> dict[str, object]:
    def counts(key: str) -> dict[str, int]:
        return dict(sorted(Counter(str(row[key]) for row in rows).items()))

    groups = defaultdict(list)
    for row in rows:
        groups[(row["phase"], row["legal_move_band"])].append(row)
    targets = []
    for (phase, band), group in sorted(groups.items()):
        targets.append({
            "phase": phase,
            "legal_move_band": band,
            "positions": len(group),
            "games": len({row["game_num"] for row in group}),
            "sample_limit": min(256, len(group)),
        })
    return {
        "positions": len(rows),
        "games": len({row["game_num"] for row in rows}),
        "phase_counts": counts("phase"),
        "side_to_move_counts": counts("side_to_move"),
        "hand_counts": counts("hand"),
        "promotion_counts": counts("promotion"),
        "legal_move_band_counts": counts("legal_move_band"),
        "audit_targets": targets,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("corpus", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--annotated-output", type=Path)
    args = parser.parse_args()

    rows = [json.loads(line) for line in args.corpus.read_text(encoding="utf-8").splitlines() if line.strip()]
    if not rows:
        raise SystemExit("empty regression corpus")
    annotated = [classify(row) for row in rows]
    report = summarize(annotated)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    if args.annotated_output:
        args.annotated_output.parent.mkdir(parents=True, exist_ok=True)
        with args.annotated_output.open("w", encoding="utf-8") as stream:
            for row in annotated:
                stream.write(json.dumps(row, ensure_ascii=False) + "\n")
    print(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
