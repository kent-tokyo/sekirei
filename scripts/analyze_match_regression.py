#!/usr/bin/env python3
"""Summarise a Sekirei-vs-Sekirei match as NNUE regression evidence.

This is a game-record diagnostic, not an absolute playing-strength estimate.
It groups candidate-vs-baseline outcomes by candidate colour, opening SFEN,
and game length so the next training/data slice can target reproducible loss
clusters.
"""

from __future__ import annotations

import argparse
import json
import re
from collections import Counter, defaultdict
from pathlib import Path


RESULT_RE = re.compile(r"^# Result: (Engine1 Win|Engine2 Win|Draw)")
ENGINE1_RE = re.compile(r"^# Engine1: .* \((Black|White)\)")
POSITION_RE = re.compile(r"^position sfen (.+?) \d+ moves(?: (.*))?$")


def score(result: str) -> float:
    return {"Engine1 Win": 1.0, "Engine2 Win": 0.0, "Draw": 0.5}[result]


def parse_game(path: Path) -> dict[str, object] | None:
    engine1_colour = None
    result = None
    sfen = None
    ply = None
    for line in path.read_text(encoding="utf-8").splitlines():
        if engine1_colour is None:
            match = ENGINE1_RE.match(line)
            if match:
                engine1_colour = match.group(1)
        if result is None:
            match = RESULT_RE.match(line)
            if match:
                result = match.group(1)
        if sfen is None:
            match = POSITION_RE.match(line)
            if match:
                sfen = match.group(1)
                moves = match.group(2) or ""
                ply = len(moves.split())
    if result is None or engine1_colour is None or sfen is None or ply is None:
        return None
    return {
        "file": str(path),
        "result": result,
        "candidate_colour": engine1_colour,
        "opening_sfen": sfen,
        "plies": ply,
        "candidate_score": score(result),
    }


def aggregate(rows: list[dict[str, object]]) -> dict[str, object]:
    def group(key: str) -> list[dict[str, object]]:
        buckets: dict[str, list[dict[str, object]]] = defaultdict(list)
        for row in rows:
            buckets[str(row[key])].append(row)
        output = []
        for value, items in buckets.items():
            output.append(
                {
                    "key": value,
                    "games": len(items),
                    "score": round(sum(float(item["candidate_score"]) for item in items) / len(items), 6),
                    "wins": sum(item["result"] == "Engine1 Win" for item in items),
                    "draws": sum(item["result"] == "Draw" for item in items),
                    "losses": sum(item["result"] == "Engine2 Win" for item in items),
                    "mean_plies": round(sum(int(item["plies"]) for item in items) / len(items), 3),
                }
            )
        return sorted(output, key=lambda item: (-item["games"], item["key"]))

    result_counts = Counter(str(row["result"]) for row in rows)
    return {
        "games": len(rows),
        "candidate_score": round(sum(float(row["candidate_score"]) for row in rows) / len(rows), 6),
        "result_counts": dict(sorted(result_counts.items())),
        "candidate_colour": group("candidate_colour"),
        "opening_sfen": group("opening_sfen"),
        "length_bucket": group("length_bucket"),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kifu_dirs", type=Path, nargs="+")
    parser.add_argument("--json-out", type=Path)
    args = parser.parse_args()

    rows = []
    for kifu_dir in args.kifu_dirs:
        for path in sorted(kifu_dir.glob("game*.txt")):
            row = parse_game(path)
            if row is None:
                continue
            plies = int(row["plies"])
            row["length_bucket"] = "short" if plies < 80 else "medium" if plies < 160 else "long"
            rows.append(row)
    if not rows:
        raise SystemExit("no complete kifu records found")

    report = aggregate(rows)
    report["kifu_dirs"] = [str(path) for path in args.kifu_dirs]
    report["records"] = rows
    rendered = json.dumps(report, ensure_ascii=False, indent=2) + "\n"
    if args.json_out:
        args.json_out.parent.mkdir(parents=True, exist_ok=True)
        args.json_out.write_text(rendered, encoding="utf-8")
    print(rendered, end="")


if __name__ == "__main__":
    main()
