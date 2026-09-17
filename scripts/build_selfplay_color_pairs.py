#!/usr/bin/env python3
"""Record board-colour pairs from a completed local self-play run.

The table is descriptive only.  Both sides use the same engine in this run,
so it cannot demonstrate a strength or colour advantage; it prevents that
confound from being silently lost during C5c evaluator analysis.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from collections import defaultdict
from pathlib import Path


ENGINE1_COLOUR = re.compile(r"^# Engine1: .* \((Black|White)\)$")


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def game_number(identifier: str) -> int:
    match = re.fullmatch(r"game(\d{4})", identifier)
    if not match:
        raise ValueError(f"invalid self-play id {identifier!r}")
    return int(match.group(1))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--review-manifest", type=Path, required=True)
    parser.add_argument("--run-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    review = json.loads(args.review_manifest.read_text(encoding="utf-8"))
    opening_by_game = {int(game["game_number"]): game["initial_sfen"] for game in review.get("games", [])}
    results = {}
    results_path = args.run_dir / "result.jsonl"
    for raw in results_path.read_text(encoding="utf-8").splitlines():
        row = json.loads(raw)
        results[game_number(row["id"])] = row["result"]
    groups: dict[str, list[dict]] = defaultdict(list)
    for number, opening in sorted(opening_by_game.items()):
        path = args.run_dir / "usi_kifu" / f"game{number:04d}.txt"
        first = path.read_text(encoding="utf-8").splitlines()[0]
        match = ENGINE1_COLOUR.fullmatch(first)
        if not match:
            raise ValueError(f"{path}: missing Engine1 colour header")
        engine1_colour = match.group(1).lower()
        result = results.get(number)
        if result not in {"candidate_win", "baseline_win", "draw"}:
            raise ValueError(f"{results_path}: missing/invalid result for game {number}")
        winner_colour = None if result == "draw" else engine1_colour if result == "candidate_win" else ("white" if engine1_colour == "black" else "black")
        groups[opening].append({"game_number": number, "engine1_colour": engine1_colour, "result": result, "winner_colour": winner_colour})
    pairs = []
    for opening, games in sorted(groups.items()):
        colours = {game["engine1_colour"] for game in games}
        pairs.append({"opening": opening, "games": games, "engine1_colours": sorted(colours), "paired": colours == {"black", "white"}})
    document = {
        "schema": "sekirei.selfplay-colour-pairs.v1",
        "status": "descriptive_only",
        "strength_claim": False,
        "inputs": {
            "review_manifest": str(args.review_manifest), "review_manifest_sha256": sha256(args.review_manifest),
            "result_jsonl": str(results_path), "result_jsonl_sha256": sha256(results_path),
        },
        "summary": {"openings": len(pairs), "paired_openings": sum(pair["paired"] for pair in pairs), "unpaired_openings": sum(not pair["paired"] for pair in pairs)},
        "pairs": pairs,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(document["summary"], sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
