#!/usr/bin/env python3
"""Freeze Q28's source-distribution-only candidate family before selection."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


SCHEMA = "sekirei.q28-tactical-source-family-preregistration.v1"


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def bind(path: Path) -> dict[str, str]:
    return {"path": str(path), "sha256": sha256(path)}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def preregister(args: argparse.Namespace) -> dict[str, object]:
    baseline = json.loads(args.baseline_audit.read_text(encoding="utf-8"))
    pool = json.loads(args.pool_scan.read_text(encoding="utf-8"))
    require(
        baseline.get("schema") == "sekirei.q28-source-distribution-audit.v1"
        and baseline.get("status") == "complete",
        "baseline audit is invalid",
    )
    require(
        pool.get("schema") == "sekirei.q28-tactical-pool-scan.v1" and pool.get("status") == "complete",
        "pool scan is invalid",
    )
    require(pool.get("selection_used_scores") is False, "pool scan used scores")
    require(args.minimum_nonchecking_train_per_stratum == 2, "Q28 fixes the minimum at two")
    return {
        "schema": SCHEMA,
        "status": "frozen_before_score_blind_parent_selection",
        "diagnostic_only": True,
        "strength_claim": False,
        "single_factor": "source distribution: at least two nonchecking parents in each train phase/material stratum",
        "inputs": {
            "baseline_score_free_audit": bind(args.baseline_audit),
            "unused_score_free_pool": bind(args.pool_scan),
            "q21x_recipe_reference": bind(args.q21x_recipe),
        },
        "source_selection": {
            "seed": args.seed,
            "train_parents_per_stratum": 8,
            "holdout_parents_per_stratum": 2,
            "minimum_nonchecking_train_per_stratum": args.minimum_nonchecking_train_per_stratum,
            "nonchecking_classes": ["capture_resource", "evasion", "quiet"],
            "all_selected_csa_sources_must_be_unique": True,
            "all_selected_symmetric_positions_must_be_unique": True,
            "selection_used_scores": False,
        },
        "fixed_training_contract": {
            "teacher": "self depth-7 exact root labels",
            "architecture": "L1=256/L2=32 default features",
            "output": "residual-material",
            "optimizer": "fresh Adam",
            "seed": 42,
            "epochs": 60,
            "learning_rate": 0.001,
            "pair_format": "existing direct top-vs-rest CP-only contract",
        },
        "holdout_boundary": {
            "candidate_must_be_frozen_before_holdout_labeling": True,
            "holdout_must_not_select_recipe_or_checkpoint": True,
        },
        "decision_contract": {
            "static": "direct top regret improves at least 10 percent and major regret does not increase",
            "same_time": "candidate versus material passes under the pre-existing fixed contract",
            "q27": "authorized only after both static and same-time pass",
            "q20": "unauthorized until Q27 passes",
        },
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline-audit", type=Path, required=True)
    parser.add_argument("--pool-scan", type=Path, required=True)
    parser.add_argument("--q21x-recipe", type=Path, required=True)
    parser.add_argument("--seed", type=int, default=2802)
    parser.add_argument("--minimum-nonchecking-train-per-stratum", type=int, default=2)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        parser.error("output already exists; preregistration is immutable")
    result = preregister(args)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
