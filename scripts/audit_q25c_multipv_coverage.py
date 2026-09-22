#!/usr/bin/env python3
"""Audit whether a wider free-MultiPV protocol can cover Q25c failures.

This is diagnostic-only and accepts only already-opened Q25c train rows.  It
never changes their labels or opens the Q25c hold-out.  A future family must
reserve a new score-blind train/hold-out boundary before using the result.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
from typing import Any

from prepare_q25_external_teacher_calibration import bind, sha256
from run_q25_external_teacher_calibration import external_search


SCHEMA = "sekirei.q25c-multipv-coverage-audit.v1"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def ordinary(score: dict[str, Any], limit: int) -> bool:
    return score.get("kind") == "cp" and isinstance(score.get("value"), int) and abs(score["value"]) <= limit


def stable_labels(results: list[dict[str, Any]], candidates: list[str], limit: int) -> dict[str, dict[str, Any]] | None:
    if len(results) != 2 or any(result.get("completion") != "search_completed" for result in results):
        return None
    first = {line.get("move"): line.get("score") for line in results[0].get("lines", [])}
    second = {line.get("move"): line.get("score") for line in results[1].get("lines", [])}
    if first != second or results[0].get("bestmove") != results[1].get("bestmove"):
        return None
    return {
        move: score
        for move in candidates
        if (score := first.get(move)) is not None and ordinary(score, limit)
    }


def atomic_write(path: Path, value: dict[str, Any]) -> None:
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    os.replace(temporary, path)


def run(args: argparse.Namespace) -> dict[str, Any]:
    prereg = json.loads(args.preregistration.read_text(encoding="utf-8"))
    labels = json.loads(args.labels.read_text(encoding="utf-8"))
    require(prereg.get("status") == "frozen_before_any_external_or_self_label", "unexpected Q25c preregistration")
    require(labels.get("status") == "complete", "Q25c train labels are incomplete")
    require(args.multipv > int(prereg["external_teacher"]["multipv"]), "audit must widen MultiPV")
    require(args.timeout > 0, "timeout must be positive")
    require(prereg["inputs"]["external_engine"]["sha256"] == sha256(args.external_engine), "external engine SHA mismatch")
    require(prereg["inputs"]["external_weights"]["sha256"] == sha256(args.external_weights), "external weights SHA mismatch")
    selected = [
        row
        for row in labels["rows"]
        if row.get("label_status") == "complete" and row.get("ordinary_labeled_move_count", 0) < 2
    ]
    output = {
        "schema": SCHEMA,
        "status": "in_progress",
        "diagnostic_only": True,
        "strength_claim": False,
        "purpose": "estimate wider free-MultiPV coverage on already-opened Q25c train failures",
        "inputs": {"preregistration": bind(args.preregistration), "labels": bind(args.labels)},
        "external_teacher": {
            "engine": bind(args.external_engine),
            "weights": bind(args.external_weights),
            "depth": prereg["candidate_contract"]["external_depth"],
            "threads": prereg["external_teacher"]["threads"],
            "multipv": args.multipv,
            "repeats": 2,
        },
        "ordinary_score_abs_max_cp": prereg["candidate_contract"]["normal_score_abs_max_cp"],
        "selected_parent_count": len(selected),
        "rows": [],
    }
    if args.output.exists():
        output = json.loads(args.output.read_text(encoding="utf-8"))
        require(output.get("schema") == SCHEMA and output["inputs"]["labels"]["sha256"] == sha256(args.labels), "output contract mismatch")
        require(output["external_teacher"]["multipv"] == args.multipv, "output MultiPV mismatch")
    completed = {row["id"] for row in output["rows"]}
    options = dict(prereg["external_teacher"]["usi_options"])
    options["MultiPV"] = str(args.multipv)
    for row in selected:
        if row["id"] in completed:
            continue
        searches = [
            external_search(
                args.external_engine,
                args.external_engine.parent,
                options,
                row["sfen"],
                prereg["candidate_contract"]["external_depth"],
                args.multipv,
                args.timeout,
            )
            for _ in range(2)
        ]
        candidates = row.get("candidate_moves", [])
        stable = stable_labels(searches, candidates, output["ordinary_score_abs_max_cp"])
        output["rows"].append(
            {
                "id": row["id"],
                "category": row["category"],
                "candidate_moves": candidates,
                "baseline_ordinary_labeled_move_count": row.get("ordinary_labeled_move_count", 0),
                "aa_stable": stable is not None,
                "ordinary_labeled_moves": stable or {},
                "ordinary_labeled_move_count": len(stable or {}),
                "searches": searches,
            }
        )
        atomic_write(args.output, output)
        print(f"q25c coverage audit: {len(output['rows'])}/{len(selected)} parents", flush=True)
    output["status"] = "complete"
    output["summary"] = {
        "aa_stable_parents": sum(row["aa_stable"] for row in output["rows"]),
        "parents_with_two_ordinary_labels": sum(row["ordinary_labeled_move_count"] >= 2 for row in output["rows"]),
    }
    atomic_write(args.output, output)
    return output


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preregistration", type=Path, required=True)
    parser.add_argument("--labels", type=Path, required=True)
    parser.add_argument("--external-engine", type=Path, required=True)
    parser.add_argument("--external-weights", type=Path, required=True)
    parser.add_argument("--multipv", type=int, default=32)
    parser.add_argument("--timeout", type=float, default=120.0)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = run(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError, RuntimeError) as error:
        parser.error(str(error))
    print(json.dumps({"status": result["status"], **result["summary"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
