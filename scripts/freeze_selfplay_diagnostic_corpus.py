#!/usr/bin/env python3
"""Freeze a small, provenance-preserving self-play diagnostic corpus.

The input is the bounded development-only selection emitted by
``build_selfplay_ledger.py``.  This script deliberately does not re-rank by
new search results: its purpose is to make the C5c evaluator experiment
reproducible before any evaluator is run.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter
from pathlib import Path
from typing import Any


CATEGORY_QUOTAS = {
    "control": 1,
    "evaluation_swing": 1,
    "check_evasion": 3,
    "material_loss": 4,
    "zero_score": 3,
}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def identity(row: dict[str, Any]) -> tuple[str, int, int]:
    return (str(row["source_id"]), int(row["game_number"]), int(row["ply"]))


def validate_row(row: dict[str, Any], source: Path) -> None:
    required = ("source_id", "game_number", "ply", "opening", "pre_move_sfen", "history_before_usi", "actual_move_csa", "categories")
    missing = [name for name in required if name not in row]
    if missing:
        raise ValueError(f"{source}: diagnostic row missing {', '.join(missing)}")
    if row.get("already_in_teacher_cache") is not False:
        raise ValueError(f"{source}: refusing teacher-cache-overlapping diagnostic row {identity(row)}")
    if not isinstance(row["pre_move_sfen"], str) or len(row["pre_move_sfen"].split()) != 4:
        raise ValueError(f"{source}: invalid SFEN for {identity(row)}")
    if not isinstance(row["history_before_usi"], list) or not all(isinstance(move, str) for move in row["history_before_usi"]):
        raise ValueError(f"{source}: invalid history for {identity(row)}")
    if not isinstance(row["categories"], list) or not row["categories"]:
        raise ValueError(f"{source}: missing categories for {identity(row)}")


def choose(rows: list[dict[str, Any]], source: Path) -> list[dict[str, Any]]:
    for row in rows:
        validate_row(row, source)
    selected: list[dict[str, Any]] = []
    selected_keys: set[tuple[str, int, int]] = set()
    per_opening: Counter[str] = Counter()
    # Stable ordering means the source document plus the published quotas fully
    # determine the selection.  The category order deliberately prioritizes the
    # rare causal candidates before the common zero-score plateau samples.
    for category, quota in CATEGORY_QUOTAS.items():
        added = 0
        for row in rows:
            key = identity(row)
            if category not in row["categories"] or key in selected_keys or per_opening[str(row["opening"])] >= 2:
                continue
            selected.append({**row, "selection_category": category})
            selected_keys.add(key)
            per_opening[str(row["opening"])] += 1
            added += 1
            if added == quota:
                break
        if added != quota:
            raise ValueError(f"{source}: only found {added}/{quota} usable rows for {category}")
    if len(selected) != sum(CATEGORY_QUOTAS.values()):
        raise AssertionError("diagnostic quota accounting failed")
    return selected


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, required=True, help="diagnostic-positions.json from one healthy run")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    document = json.loads(args.input.read_text(encoding="utf-8"))
    if document.get("schema") != "sekirei.selfplay-diagnostic-positions.v1":
        raise SystemExit(f"{args.input}: unsupported schema")
    if document.get("status") != "development_only":
        raise SystemExit(f"{args.input}: expected development_only input")
    rows = document.get("positions")
    if not isinstance(rows, list):
        raise SystemExit(f"{args.input}: positions must be a list")
    selected = choose(rows, args.input)
    output = {
        "schema": "sekirei.selfplay-fixed-diagnostic-corpus.v1",
        "status": "development_only",
        "strength_claim": False,
        "purpose": "fixed 20k material-versus-healthy-NNUE diagnostic; not a training or Elo corpus",
        "input": {"path": str(args.input), "sha256": sha256(args.input), "positions_available": len(rows)},
        "selection": {
            "category_quotas": CATEGORY_QUOTAS,
            "per_opening_max": 2,
            "teacher_cache_overlap_required": 0,
            "selected_positions": len(selected),
        },
        "positions": selected,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(output, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(output["selection"], ensure_ascii=False, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
