#!/usr/bin/env python3
"""Freeze Q21n completion with one twice-timed-out, unlabeled parent censored."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def bind(path: Path) -> dict[str, str]:
    return {"path": str(path), "sha256": sha256(path)}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def timeout_without_label(row: dict[str, Any]) -> bool:
    result = row.get("teacher_root", {})
    return (
        row.get("candidate_prefix_complete") is not True
        and row.get("complete_legal_root_set") is not True
        and result.get("completion") == "timeout"
        and result.get("score_cp") is None
        and not result.get("root_candidates")
    )


def prepare(args: argparse.Namespace) -> dict[str, Any]:
    prereg = json.loads(args.preregistration.read_text(encoding="utf-8"))
    corpus = json.loads(args.corpus.read_text(encoding="utf-8"))
    initial = json.loads(args.initial_deep.read_text(encoding="utf-8"))
    retry_amendment = json.loads(args.retry_amendment.read_text(encoding="utf-8"))
    retry = json.loads(args.retry_deep.read_text(encoding="utf-8"))
    require(
        prereg.get("schema") == "sekirei.q21n-teacher-depth-preregistration.v1",
        "unexpected Q21n preregistration",
    )
    require(
        retry_amendment.get("schema") == "sekirei.q21n-timeout-retry-amendment.v1"
        and retry_amendment.get("status") == "frozen_before_retry_label",
        "unexpected retry amendment",
    )
    target = retry_amendment["retry"]["position_id"]
    initial_incomplete = [row for row in initial.get("rows", []) if timeout_without_label(row)]
    require(
        len(initial.get("rows", [])) == 18
        and len(initial_incomplete) == 1
        and initial_incomplete[0].get("id") == target,
        "initial run must have exactly the preregistered unlabeled timeout",
    )
    require(
        len(retry.get("rows", [])) == 1
        and retry["rows"][0].get("id") == target
        and timeout_without_label(retry["rows"][0]),
        "retry must be the same parent and remain an unlabeled timeout",
    )
    completed = [
        row for row in initial["rows"]
        if row.get("candidate_prefix_complete") is True
        and row.get("complete_legal_root_set") is True
    ]
    require(len(completed) == 17, "Q21n timeout exclusion requires 17 complete parents")
    positions = {row["id"]: row for row in corpus.get("positions", [])}
    require(target in positions, "timeout parent missing from corpus")
    accepted_categories = [positions[row["id"]]["category"] for row in completed]
    require(
        sum(category.endswith("/balanced") for category in accepted_categories) == 6,
        "timeout exclusion must retain every balanced parent",
    )
    require(
        len(set(accepted_categories)) == 9,
        "timeout exclusion must retain every phase/material stratum",
    )
    return {
        "schema": "sekirei.q21n-timeout-exclusion.v1",
        "status": "frozen_before_completed-score-inspection",
        "diagnostic_only": True,
        "strength_claim": False,
        "reason": (
            "one 479-legal-move parent returned no score or ranking at both "
            "the 300s initial limit and the preregistered 900s retry limit"
        ),
        "inputs": {
            "preregistration": bind(args.preregistration),
            "corpus": bind(args.corpus),
            "initial_deep": bind(args.initial_deep),
            "retry_amendment": bind(args.retry_amendment),
            "retry_deep": bind(args.retry_deep),
        },
        "completion_contract": {
            "accepted_complete_parents": 17,
            "resource_censored_parents": 1,
            "excluded_position_id": target,
            "excluded_category": positions[target]["category"],
            "excluded_due_to_score_or_rank": False,
            "balanced_parents_retained": 6,
            "all_phase_material_strata_retained": True,
            "substitution_allowed": False,
            "original_decision_thresholds_unchanged": True,
            "development_match_authorized": False,
            "q20_authorized": False,
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preregistration", type=Path, required=True)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--initial-deep", type=Path, required=True)
    parser.add_argument("--retry-amendment", type=Path, required=True)
    parser.add_argument("--retry-deep", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        document = prepare(args)
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.write_text(
        json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps({"status": document["status"], **document["completion_contract"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
