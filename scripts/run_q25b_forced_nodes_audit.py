#!/usr/bin/env python3
"""Audit fixed-node forced external labels without opening Q25a's hold-out."""

from __future__ import annotations

import argparse
import json
from collections import Counter
from pathlib import Path
from typing import Any

from prepare_q25_external_teacher_calibration import bind, sha256
from run_q25_external_teacher_calibration import external_search


SCHEMA = "sekirei.q25b-forced-nodes-audit.v1"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def stable(results: list[dict[str, Any]], move: str, nodes: int) -> bool:
    if len(results) != 2 or any(result.get("completion") != "search_completed" for result in results):
        return False
    signatures = []
    for result in results:
        lines = result.get("lines", [])
        if result.get("bestmove") != move or len(lines) != 1 or lines[0].get("move") != move:
            return False
        if result.get("requested_nodes") != nodes or not isinstance(lines[0].get("nodes"), int):
            return False
        signatures.append((lines[0].get("score"), lines[0].get("nodes")))
    return signatures[0] == signatures[1]


def ordinary(result: dict[str, Any]) -> bool:
    score = result["lines"][0].get("score", {})
    return score.get("kind") == "cp" and isinstance(score.get("value"), int) and abs(score["value"]) <= 10_000


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preregistration", type=Path, required=True)
    parser.add_argument("--train-labels", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--nodes", type=int, default=50_000)
    parser.add_argument("--timeout", type=float, default=90.0)
    parser.add_argument("--offset", type=int, default=0)
    parser.add_argument("--limit", type=int)
    args = parser.parse_args()
    require(args.nodes > 0 and args.timeout > 0, "nodes and timeout must be positive")
    prereg = json.loads(args.preregistration.read_text(encoding="utf-8"))
    labels = json.loads(args.train_labels.read_text(encoding="utf-8"))
    require(prereg.get("schema") == "sekirei.q25a-external-label-execution-preregistration.v1", "unexpected preregistration")
    require(labels.get("schema") == "sekirei.q25a-external-label-measurements.v2" and labels.get("kind") == "train", "unexpected labels")
    require(labels.get("preregistration", {}).get("sha256") == sha256(args.preregistration), "labels not bound to preregistration")
    rows = labels.get("rows", [])
    by_category: dict[str, dict[str, Any]] = {}
    for row in rows:
        by_category.setdefault(row["category"], row)
    require(len(by_category) == 9, "Q25b requires one frozen train parent from every stratum")

    external = prereg["external_teacher"]
    engine = Path(prereg["inputs"]["external_engine"]["path"])
    weights = Path(prereg["inputs"]["external_weights"]["path"])
    require(engine.is_file() and weights.is_file(), "pinned external assets are unavailable")
    require(sha256(engine) == prereg["inputs"]["external_engine"]["sha256"], "external engine SHA mismatch")
    require(sha256(weights) == prereg["inputs"]["external_weights"]["sha256"], "external weights SHA mismatch")

    all_selected = [by_category[category] for category in sorted(by_category)]
    selected = all_selected[args.offset : None if args.limit is None else args.offset + args.limit]
    require(selected, "selected Q25b parent slice is empty")
    preregistration = {
        "schema": SCHEMA,
        "status": "frozen_before_measurement",
        "diagnostic_only": True,
        "strength_claim": False,
        "contract": {
            "parents": 9,
            "slice": {"offset": args.offset, "limit": args.limit, "selected": len(selected)},
            "selection": "first existing Q25a frozen train parent in each of 9 strata",
            "limit": {"kind": "nodes", "value": args.nodes},
            "repeats": 2,
            "multipv": 1,
            "acceptance": "forced bestmove, reported score, and reported node count must match A/A",
            "holdout": "sealed and not read",
        },
        "inputs": {"q25a_preregistration": bind(args.preregistration), "q25a_train_labels": bind(args.train_labels), "engine": bind(engine), "weights": bind(weights)},
        "parents": [{"id": row["id"], "category": row["category"], "candidate_moves": row["candidate_moves"]} for row in selected],
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    prereg_path = args.output.with_name("preregistration.json")
    prereg_path.write_text(json.dumps(preregistration, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    measured = []
    for parent in selected:
        attempts: dict[str, list[dict[str, Any]]] = {}
        for move in parent["candidate_moves"]:
            attempts[move] = [
                external_search(engine, engine.parent, external["usi_options"], parent["sfen"], None, 1, args.timeout, searchmove=move, nodes=args.nodes)
                for _ in range(2)
            ]
        valid = {move: stable(results, move, args.nodes) for move, results in attempts.items()}
        ordinary_count = sum(ordinary(results[0]) for move, results in attempts.items() if valid[move])
        measured.append({"id": parent["id"], "category": parent["category"], "candidate_moves": parent["candidate_moves"], "attempts": attempts, "stable_by_move": valid, "ordinary_labeled_move_count": ordinary_count})

    full_moves = sum(len(row["candidate_moves"]) for row in measured)
    stable_moves = sum(sum(row["stable_by_move"].values()) for row in measured)
    usable_parents = sum(row["ordinary_labeled_move_count"] >= 2 for row in measured)
    result = {
        "schema": SCHEMA,
        "status": (
            "partial"
            if len(measured) != len(all_selected)
            else "protocol_pass" if stable_moves == full_moves and usable_parents == len(measured) else "protocol_fail"
        ),
        "diagnostic_only": True,
        "strength_claim": False,
        "preregistration": bind(prereg_path),
        "coverage": {"parents": len(measured), "candidate_moves": full_moves, "aa_stable_moves": stable_moves, "parents_with_two_ordinary_labels": usable_parents, "by_stratum": dict(Counter(row["category"] for row in measured))},
        "rows": measured,
        "next_action": "separately preregister a full 72-parent family only on protocol_pass; do not open the Q25a hold-out in either outcome",
    }
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"status": result["status"], **result["coverage"]}, ensure_ascii=False, sort_keys=True))
    return 0 if result["status"] in {"protocol_pass", "partial"} else 1


if __name__ == "__main__":
    raise SystemExit(main())
