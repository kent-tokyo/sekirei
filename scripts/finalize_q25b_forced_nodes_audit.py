#!/usr/bin/env python3
"""Merge Q25b strata batches into one protocol verdict."""

from __future__ import annotations

import argparse
import json
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inputs", type=Path, nargs="+", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    reports = [json.loads(path.read_text(encoding="utf-8")) for path in args.inputs]
    rows = [row for report in reports for row in report["rows"]]
    categories = {row["category"] for row in rows}
    ids = {row["id"] for row in rows}
    if len(rows) != 9 or len(categories) != 9 or len(ids) != 9:
        parser.error("Q25b requires nine unique strata parents")
    moves = sum(len(row["candidate_moves"]) for row in rows)
    stable = sum(sum(row["stable_by_move"].values()) for row in rows)
    ordinary = sum(row["ordinary_labeled_move_count"] >= 2 for row in rows)
    result = {
        "schema": "sekirei.q25b-forced-nodes-audit-decision.v1",
        "status": "protocol_pass" if stable == moves and ordinary == 9 else "protocol_fail",
        "diagnostic_only": True,
        "strength_claim": False,
        "inputs": [{"path": str(path)} for path in args.inputs],
        "coverage": {"parents": 9, "strata": 9, "candidate_moves": moves, "aa_stable_moves": stable, "parents_with_two_ordinary_labels": ordinary},
        "holdout_inspected": False,
        "candidate_trained": False,
        "q27_authorized": False,
        "next_action": "do not label all 72 parents or train; fixed-node forced labels are not A/A-stable across the frozen strata sample",
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(result, ensure_ascii=False, sort_keys=True))
    return 0 if result["status"] == "protocol_pass" else 1


if __name__ == "__main__":
    raise SystemExit(main())
