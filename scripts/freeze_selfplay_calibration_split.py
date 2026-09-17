#!/usr/bin/env python3
"""Freeze C5c-disjoint train/hold-out positions from one healthy self-play run.

This is deliberately position-disjoint, not opening-disjoint: five of the six
available validation opening groups contain a C5c position.  The manifest
records that limitation so the resulting pilot cannot be promoted as an
opening-generalisation result.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter
from pathlib import Path
from typing import Any


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def canonical_sfen(sfen: str) -> str:
    fields = sfen.split()
    if len(fields) != 4:
        raise ValueError(f"invalid SFEN {sfen!r}")
    return " ".join(fields[:3])


def phase(ply: int) -> str:
    return "opening" if ply < 40 else "middlegame" if ply < 120 else "endgame"


def sample(game: dict[str, Any], row: dict[str, Any]) -> dict[str, Any]:
    ply = int(row["ply"])
    return {
        "schema_version": 1,
        "sfen": row["pre_move_sfen"],
        "source": {"kind": "selfplay-replay", "path": game["replay_path"], "game_number": game["game_number"], "ply": ply, "opening": game["opening"]},
        "tags": {"phase": phase(ply), "side_to_move": row["side_to_move"], "in_check": row.get("side_to_move_in_check") is True, "has_capture": row.get("captured_piece") is not None},
    }


def write_jsonl(path: Path, rows: list[dict[str, Any]]) -> None:
    path.write_text("".join(json.dumps(row, ensure_ascii=False, sort_keys=True) + "\n" for row in rows), encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ledger", type=Path, required=True)
    parser.add_argument("--c5c-positions", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    ledger = json.loads(args.ledger.read_text(encoding="utf-8"))
    c5c = json.loads(args.c5c_positions.read_text(encoding="utf-8"))
    if ledger.get("schema") != "sekirei.selfplay-representative-ledger.v1" or ledger.get("status") != "development_only":
        parser.error("ledger must be a development-only representative ledger")
    if c5c.get("schema") != "sekirei.selfplay-diagnostic-positions.v1" or c5c.get("status") != "development_only":
        parser.error("c5c positions must be development-only diagnostic positions")
    c5c_rows = c5c.get("positions", [])
    if len(c5c_rows) != 32:
        parser.error("expected exactly 32 C5c position records")
    # Identical SFENs can arise in separate self-play games; exclude the
    # canonical position globally, while retaining both record and SFEN counts.
    c5c_sfens = {canonical_sfen(row["pre_move_sfen"]) for row in c5c_rows}
    games = ledger.get("representative_games", [])
    if not games:
        parser.error("representative ledger is empty")
    train, holdout = [], []
    held_games, held_openings, c5c_excluded = set(), set(), 0
    for game in games:
        replay_path = Path(game["replay_path"])
        replay = json.loads(replay_path.read_text(encoding="utf-8"))
        target = train if game["split"] == "train" else holdout
        for row in replay.get("positions", []):
            key = canonical_sfen(row["pre_move_sfen"])
            if key in c5c_sfens:
                c5c_excluded += 1
                continue
            target.append(sample(game, row))
            if game["split"] == "validation":
                held_games.add(int(game["game_number"]))
                held_openings.add(game["opening"])
    train_sfens = {canonical_sfen(row["sfen"]) for row in train}
    overlapping = {canonical_sfen(row["sfen"]) for row in holdout} & train_sfens
    if overlapping:
        holdout = [row for row in holdout if canonical_sfen(row["sfen"]) not in overlapping]
    if not train or not holdout:
        parser.error("C5c filtering left an empty train or hold-out set")
    args.output_dir.mkdir(parents=True, exist_ok=True)
    train_path = args.output_dir / "train-positions.jsonl"
    holdout_path = args.output_dir / "holdout-positions.jsonl"
    write_jsonl(train_path, train)
    write_jsonl(holdout_path, holdout)
    document = {
        "schema": "sekirei.selfplay-calibration-split.v1", "status": "frozen", "strength_claim": False,
        "split_kind": "position_disjoint_not_opening_disjoint",
        "limitation": "C5c used positions from five of six validation opening groups; this hold-out is independent by position/SFEN, not by opening family.",
        "inputs": {"ledger": str(args.ledger), "ledger_sha256": sha256(args.ledger), "c5c_positions": str(args.c5c_positions), "c5c_positions_sha256": sha256(args.c5c_positions)},
        "exclusions": {"c5c_position_records": len(c5c_rows), "c5c_unique_sfens": len(c5c_sfens), "rows_excluded_by_c5c_sfen": c5c_excluded, "cross_split_sfen_overlap_removed_from_holdout": len(overlapping)},
        "train": {"path": str(train_path), "sha256": sha256(train_path), "positions": len(train), "games": len({row["source"]["game_number"] for row in train})},
        "holdout": {"path": str(holdout_path), "sha256": sha256(holdout_path), "positions": len(holdout), "games": len(held_games), "opening_groups": len(held_openings), "game_numbers": sorted(held_games), "phase_counts": dict(Counter(row["tags"]["phase"] for row in holdout))},
    }
    manifest = args.output_dir / "manifest.json"
    manifest.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"train_positions": len(train), "holdout_positions": len(holdout), "holdout_games": len(held_games), "holdout_opening_groups": len(held_openings), "c5c_excluded_rows": c5c_excluded, "cross_split_overlap_removed": len(overlapping)}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
