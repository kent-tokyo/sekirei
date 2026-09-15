#!/usr/bin/env python3
"""Validate a fixed-budget diagnostic comparison artifact."""
from __future__ import annotations

import json
import sys
from pathlib import Path


SCHEMA = "sekirei.floodgate-budget-comparison.v1"


def validate(document: dict) -> list[str]:
    errors = []
    if document.get("schema") != SCHEMA:
        errors.append("schema")
    if document.get("diagnostic_only") is not True:
        errors.append("diagnostic_only")
    claims = document.get("claims")
    if not isinstance(claims, dict) or claims.get("strength") != "not_permitted":
        errors.append("claims.strength")
    for side in ("baseline", "candidate"):
        value = document.get(side)
        if not isinstance(value, dict):
            errors.append(side)
        elif not isinstance(value.get("nodes"), int) or value["nodes"] < 0:
            errors.append(f"{side}.nodes")
    rows = document.get("rows")
    if not isinstance(rows, list):
        return errors + ["rows"]
    keys = set()
    comparable = 0
    for index, row in enumerate(rows):
        prefix = f"rows[{index}]"
        if not isinstance(row, dict):
            errors.append(prefix)
            continue
        item_key = (row.get("game_id"), row.get("ply"))
        if item_key in keys:
            errors.append(f"{prefix}.duplicate_key")
        keys.add(item_key)
        if not isinstance(row.get("game_id"), str) or not row["game_id"]:
            errors.append(f"{prefix}.game_id")
        if not isinstance(row.get("ply"), int) or row["ply"] < 0:
            errors.append(f"{prefix}.ply")
        if not isinstance(row.get("comparable"), bool):
            errors.append(f"{prefix}.comparable")
        if row.get("comparable"):
            comparable += 1
            for key in ("bestmove_changed", "score_delta_candidate_minus_baseline_cp",
                        "depth_delta_candidate_minus_baseline"):
                if row.get(key) is None:
                    errors.append(f"{prefix}.{key}")
        elif row.get("bestmove_changed") is not None:
            errors.append(f"{prefix}.incomplete_bestmove")
    summary = document.get("summary")
    if not isinstance(summary, dict):
        errors.append("summary")
    else:
        if summary.get("total") != len(rows):
            errors.append("summary.total")
        if summary.get("comparable") != comparable:
            errors.append("summary.comparable")
        if summary.get("incomplete") != len(rows) - comparable:
            errors.append("summary.incomplete")
    return errors


def main(argv=None) -> int:
    args = argv or sys.argv[1:]
    if len(args) != 1:
        print(f"usage: {Path(sys.argv[0]).name} COMPARISON.json", file=sys.stderr)
        return 2
    document = json.loads(Path(args[0]).read_text(encoding="utf-8"))
    errors = validate(document)
    if errors:
        print("invalid budget comparison: " + ", ".join(errors), file=sys.stderr)
        return 1
    print(f"valid budget comparison: {args[0]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
