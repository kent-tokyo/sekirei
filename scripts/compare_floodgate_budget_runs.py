#!/usr/bin/env python3
"""Compare two fixed-node Floodgate diagnostic runs without strength claims."""
from __future__ import annotations

import argparse
import json
from pathlib import Path


SCHEMA = "sekirei.floodgate-core-diagnostic.v1"


def key(row: dict) -> tuple[str, int]:
    source = row.get("source", {})
    return str(source.get("game_id", "")), int(source.get("ply", -1))


def load(path: Path) -> dict:
    document = json.loads(path.read_text(encoding="utf-8"))
    if document.get("schema") != SCHEMA or document.get("diagnostic_only") is not True:
        raise ValueError(f"{path}: not a diagnostic-only core report")
    rows = document.get("results")
    if not isinstance(rows, list) or not rows:
        raise ValueError(f"{path}: results must be a non-empty list")
    indexed = {key(row): row for row in rows}
    if len(indexed) != len(rows) or any(k[0] == "" or k[1] < 0 for k in indexed):
        raise ValueError(f"{path}: duplicate or incomplete source key")
    return {"document": document, "rows": indexed}


def compare(baseline_path: Path, candidate_path: Path) -> dict:
    baseline = load(baseline_path)
    candidate = load(candidate_path)
    if set(baseline["rows"]) != set(candidate["rows"]):
        raise ValueError("baseline/candidate source sets differ")
    rows = []
    for item_key in sorted(baseline["rows"]):
        before = baseline["rows"][item_key]
        after = candidate["rows"][item_key]
        before_complete = before.get("completion") == "search_completed"
        after_complete = after.get("completion") == "search_completed"
        rows.append({
            "game_id": item_key[0],
            "ply": item_key[1],
            "baseline_completion": before.get("completion", "unknown"),
            "candidate_completion": after.get("completion", "unknown"),
            "comparable": before_complete and after_complete,
            "bestmove_changed": before.get("bestmove") != after.get("bestmove")
            if before_complete and after_complete else None,
            "score_delta_candidate_minus_baseline_cp": after.get("score_cp") - before.get("score_cp")
            if before_complete and after_complete and isinstance(before.get("score_cp"), int)
            and isinstance(after.get("score_cp"), int) else None,
            "depth_delta_candidate_minus_baseline": after.get("depth") - before.get("depth")
            if before_complete and after_complete and isinstance(before.get("depth"), int)
            and isinstance(after.get("depth"), int) else None,
        })
    comparable = [row for row in rows if row["comparable"]]
    return {
        "schema": "sekirei.floodgate-budget-comparison.v1",
        "diagnostic_only": True,
        "baseline": {"path": str(baseline_path), "nodes": baseline["document"].get("nodes")},
        "candidate": {"path": str(candidate_path), "nodes": candidate["document"].get("nodes")},
        "rows": rows,
        "summary": {
            "total": len(rows),
            "comparable": len(comparable),
            "incomplete": len(rows) - len(comparable),
            "bestmove_changes": sum(row["bestmove_changed"] is True for row in comparable),
            "nonzero_score_changes": sum(row["score_delta_candidate_minus_baseline_cp"] not in (None, 0) for row in comparable),
        },
        "claims": {
            "strength": "not_permitted",
            "interpretation": "budget sensitivity only; not a causal or playing-strength result",
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("baseline", type=Path)
    parser.add_argument("candidate", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    report = compare(args.baseline, args.candidate)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {report['summary']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
