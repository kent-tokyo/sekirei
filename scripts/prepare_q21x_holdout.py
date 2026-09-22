#!/usr/bin/env python3
"""Freeze Q21x's candidate and hold-out runtime before hold-out labels."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

import prepare_q21x_baseline as baseline


SCHEMA = "sekirei.q21x-holdout-preregistration.v1"


def prepare(args: argparse.Namespace) -> dict[str, Any]:
    train_prereg = baseline.read(args.train_preregistration)
    decision = baseline.read(args.train_decision)
    holdout = baseline.read(args.holdout_reserve)
    baseline.require(
        train_prereg.get("schema") == baseline.SCHEMA
        and train_prereg.get("status") == "frozen_before_train_labels",
        "unexpected Q21x train preregistration",
    )
    baseline.require(
        decision.get("schema") == "sekirei.q21x-train-decision.v1"
        and decision.get("status") == "candidate_frozen"
        and decision.get("holdout_inspected") is False,
        "Q21x train candidate is not frozen before holdout",
    )
    baseline.require(
        train_prereg["inputs"]["holdout_reserve_sealed"]["sha256"]
        == baseline.sha256(args.holdout_reserve),
        "holdout membership SHA mismatch",
    )
    rows = baseline.positions(holdout, "sekirei.q21w-holdout-reserve.v1", 18)
    baseline.source_keys(rows)
    baseline.identities(rows)
    candidate = baseline.bind(args.candidate)
    baseline.require(
        decision["candidate"]["artifact"]["sha256"] == candidate["sha256"],
        "candidate SHA differs from train decision",
    )
    for name, path in (
        ("engine", args.engine),
        ("weights", args.teacher_weights),
        ("ranking_auditor", args.ranking_auditor),
    ):
        baseline.require(
            train_prereg["inputs"][name]["sha256"] == baseline.sha256(path),
            f"{name} SHA mismatch",
        )
    document = {
        "schema": SCHEMA,
        "status": "frozen_before_holdout_labels",
        "diagnostic_only": True,
        "strength_claim": False,
        "parents": 18,
        "inputs": {
            "train_preregistration": baseline.bind(args.train_preregistration),
            "train_decision": baseline.bind(args.train_decision),
            "corpus": baseline.bind(args.holdout_reserve),
            "engine": baseline.bind(args.engine),
            "weights": baseline.bind(args.teacher_weights),
            "ranking_auditor": baseline.bind(args.ranking_auditor),
            "candidate": candidate,
        },
        "candidate_contract": train_prereg["candidate_contract"],
        "teacher_contract": train_prereg["teacher_contract"],
        "validation_contract": {
            "fresh_holdout": True,
            "selection_used_holdout": False,
            "baseline": "initial fixed-T checkpoint",
            "minimum_mean_direct_top_regret_reduction": 0.10,
            "top1_matches_must_not_decrease": True,
            "major_regrets_ge_300cp_must_not_increase": True,
            "same_time_candidate_must_not_trail_material": True,
            "time_ms": 1000,
            "repeats_per_arm": 2,
        },
        "authorization": {
            "pass": "Q21x baseline complete; continue to Q21y capacity-only comparison",
            "fail": "reject baseline as a candidate; do not authorize Q20",
            "q20_authorized": False,
        },
        "tools": {
            "preparer": baseline.bind(Path(__file__).resolve()),
            "label_runner": baseline.bind(args.label_runner),
            "pair_builder": baseline.bind(args.pair_builder),
            "screen_runner": baseline.bind(args.screen_runner),
        },
    }
    args.output_dir.mkdir(parents=True, exist_ok=True)
    output = args.output_dir / "holdout-preregistration.json"
    encoded = json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    if output.exists() and output.read_text(encoding="utf-8") != encoded:
        raise ValueError("existing Q21x holdout preregistration differs")
    output.write_text(encoded, encoding="utf-8")
    return document


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--train-preregistration", type=Path, required=True)
    parser.add_argument("--train-decision", type=Path, required=True)
    parser.add_argument("--holdout-reserve", type=Path, required=True)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--teacher-weights", type=Path, required=True)
    parser.add_argument("--ranking-auditor", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--label-runner", type=Path, required=True)
    parser.add_argument("--pair-builder", type=Path, required=True)
    parser.add_argument("--screen-runner", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    try:
        document = prepare(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    print(json.dumps({"status": document["status"], "parents": document["parents"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
