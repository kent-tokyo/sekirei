#!/usr/bin/env python3
"""Freeze Q21n depth-7 adjudication for every moderate depth-5 regret."""

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


def prepare(args: argparse.Namespace) -> dict[str, Any]:
    summary = json.loads(args.summary.read_text(encoding="utf-8"))
    corpus = json.loads(args.corpus.read_text(encoding="utf-8"))
    require(
        summary.get("schema") == "sekirei.q21n-teacher-depth-audit.v1"
        and summary.get("status") == "complete"
        and summary.get("classification") == "mixed_teacher_and_student_risk",
        "depth-7 adjudication requires the completed mixed Q21n audit",
    )
    selected = [
        {
            "position_id": row["position_id"],
            "category": row["category"],
            "forcing_class": row["forcing_class"],
            "depth5_regret_cp": row["deep_regret_cp"],
            "depth3_top_moves": row["shallow_top_moves"],
        }
        for row in summary.get("rows", [])
        if row.get("ordinary_cp_comparable") is True
        and row.get("deep_regret_cp", 0) >= 100
    ]
    require(len(selected) == summary["overall"]["moderate_regret_ge_100cp"], "moderate-regret selection mismatch")
    require(0 < len(selected) <= 6, "unexpected moderate-regret parent count")
    require(all(item["depth5_regret_cp"] < 300 for item in selected), "major regret belongs to the primary classification")
    positions = {row["id"] for row in corpus.get("positions", [])}
    require(all(item["position_id"] in positions for item in selected), "selected parent missing from corpus")
    require(all(item["depth3_top_moves"] for item in selected), "selected parent lacks a depth-3 top move")
    return {
        "schema": "sekirei.q21n-depth7-adjudication-preregistration.v1",
        "status": "frozen_before_depth7_labels",
        "diagnostic_only": True,
        "strength_claim": False,
        "selection_rule": "all and only ordinary parents with preregistered depth-5 regret >=100cp and <300cp",
        "selected": selected,
        "contract": {
            "depth": 7,
            "threads": 1,
            "spec_top_n": 0,
            "cold_process_per_search": True,
            "searches_per_parent": "one free search plus every tied depth-3 top move fixed at root",
            "timeout_seconds_per_search": 600,
            "weights_sha256": sha256(args.weights),
            "binary_sha256": sha256(args.binary),
            "nnue_output": "residual-material",
        },
        "decision_rule": {
            "teacher_depth_insufficient_if": "any depth-7 regret >=300cp",
            "mixed_if": "no >=300cp regret but any depth-7 regret >=100cp or any mate-like/incomplete result",
            "student_reproduction_primary_if": "all depth-7 regrets <100cp",
            "depth7_T_is_ground_truth": False,
            "development_match_authorized": False,
            "q20_authorized": False,
        },
        "inputs": {
            "q21n_summary": bind(args.summary),
            "corpus": bind(args.corpus),
            "binary": bind(args.binary),
            "weights": bind(args.weights),
        },
        "tools": {
            "preparer": bind(Path(__file__).resolve()),
            "runner": bind(args.runner),
            "finalizer": bind(args.finalizer),
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--summary", type=Path, required=True)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--weights", type=Path, required=True)
    parser.add_argument("--runner", type=Path, required=True)
    parser.add_argument("--finalizer", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        document = prepare(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"status": document["status"], "selected": document["selected"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
