#!/usr/bin/env python3
"""Audit whether Q25e's fixed external-label update transfers by parent.

This is a diagnostic-only eight-fold cross-validation on Q25e's *train*
parents.  It never reads the opened Q25e hold-out, never selects a deployable
checkpoint, and keeps every training setting fixed.  Its purpose is to decide
whether the failed Q25e hold-out is consistent with parent memorisation before
another external-label family is considered.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
from pathlib import Path
from statistics import mean
from typing import Any

import run_q21u_train_screen as shared


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def pair_subset(document: dict[str, Any], parent_ids: set[str]) -> dict[str, Any]:
    return {
        key: value
        for key, value in document.items()
        if key
        in {
            "schema",
            "diagnostic_only",
            "strength_claim",
            "source_contract",
            "source_teacher",
            "pair_selection",
        }
    } | {"pairs": [row for row in document["pairs"] if row["parent_id"] in parent_ids]}


def folds(reserve: dict[str, Any], count: int = 8) -> list[set[str]]:
    by_category: dict[str, list[str]] = {}
    for row in reserve["positions"]:
        by_category.setdefault(row["category"], []).append(row["id"])
    require(by_category, "reserve has no positions")
    require(
        all(len(rows) == count for rows in by_category.values()),
        "Q25f requires exactly eight train parents per stratum",
    )
    result = [set() for _ in range(count)]
    for rows in by_category.values():
        for index, parent_id in enumerate(sorted(rows)):
            result[index].add(parent_id)
    return result


def run_training(
    trainer: Path,
    pairs: Path,
    initial_weights: Path,
    output: Path,
    recipe: dict[str, Any],
    timeout: float,
) -> None:
    command = [
        str(trainer),
        "--ranking-pairs",
        str(pairs),
        "--ranking-objective",
        recipe["objective"],
        "--ranking-parent-balanced",
        "--ranking-epochs",
        str(recipe["epochs"]),
        "--init-weights",
        str(initial_weights),
        "--nnue-output",
        recipe["nnue_output"],
        "--lr",
        str(recipe["learning_rate"]),
        "--seed",
        str(recipe["seed"]),
        "--output",
        str(output),
    ]
    completed = subprocess.run(
        command,
        text=True,
        capture_output=True,
        check=False,
        timeout=timeout,
        env={**os.environ, "RAYON_NUM_THREADS": "1"},
    )
    if completed.returncode:
        raise RuntimeError(f"Q25f training failed: {completed.stderr[-4000:]}")


def audit(auditor: Path, pairs: Path, weights: Path) -> dict[str, Any]:
    return shared.run_json([str(auditor), "--pairs", str(pairs), "--weights", str(weights)], 120.0)


def metrics(rows: list[dict[str, Any]]) -> dict[str, Any]:
    require(rows, "no validation parent diagnostics")
    return {
        "parents": len(rows),
        "mean_direct_top_regret_cp": mean(row["teacher_rank_loss_cp"] for row in rows),
        "top1_matches": sum(row["teacher_rank_loss_cp"] == 0 for row in rows),
        "major_regrets_ge_300cp": sum(row["teacher_rank_loss_cp"] >= 300 for row in rows),
    }


def run(args: argparse.Namespace) -> dict[str, Any]:
    family = shared.read(args.family_preregistration)
    reserve = shared.read(args.train_reserve)
    pairs = shared.read(args.train_pairs)
    require(family["family_id"].startswith("q25e-"), "expected frozen Q25e family")
    require(family["fixed_training"]["objective"] == "pairwise", "Q25f fixes pairwise training")
    reserve_ids = {row["id"] for row in reserve["positions"]}
    require(
        reserve_ids and all(parent_id.startswith("q25a-train-") for parent_id in reserve_ids),
        "Q25f accepts only the frozen Q25e train reserve",
    )
    recipe = {
        "objective": family["fixed_training"]["objective"],
        "epochs": family["fixed_training"]["epochs"],
        "learning_rate": family["fixed_training"]["learning_rate"],
        "seed": family["fixed_training"]["seed"],
        "nnue_output": family["fixed_model"]["nnue_output"],
    }
    all_ids = reserve_ids
    pair_ids = {row["parent_id"] for row in pairs["pairs"]}
    require(pair_ids == all_ids, "every frozen Q25e train parent must have pairs")
    args.output_dir.mkdir(parents=True, exist_ok=True)
    preregistration = {
        "schema": "sekirei.q25f-external-label-cv-preregistration.v1",
        "status": "frozen_before_cross_validation",
        "diagnostic_only": True,
        "strength_claim": False,
        "single_factor": "validation parent identity; Q25e external-label protocol and recipe are unchanged",
        "prohibitions": [
            "do not read Q25e holdout labels or audits",
            "do not select a deployable candidate",
            "do not authorize Q27 or Q20 from cross-validation",
        ],
        "inputs": {
            "family_preregistration": shared.bind(args.family_preregistration),
            "train_reserve": shared.bind(args.train_reserve),
            "train_pairs": shared.bind(args.train_pairs),
            "initial_weights": shared.bind(args.initial_weights),
            "trainer": shared.bind(args.trainer),
            "ranking_auditor": shared.bind(args.ranking_auditor),
        },
        "recipe": recipe,
        "folds": 8,
        "decision_contract": {
            "mean_direct_top_regret_reduction_minimum": 0.10,
            "top1_matches_must_not_decrease": True,
            "major_regrets_must_not_increase": True,
            "pass_interpretation": "diagnostic evidence only; a new score-blind family would still require a new reserve",
        },
    }
    prereg_path = args.output_dir / "preregistration.json"
    shared.atomic_write(prereg_path, preregistration)
    baseline = audit(args.ranking_auditor, args.train_pairs, args.initial_weights)
    shared.atomic_write(args.output_dir / "baseline-train-audit.json", baseline)
    baseline_metrics = metrics(baseline["model_diagnostic"]["parent_diagnostics"])
    validation_rows: list[dict[str, Any]] = []
    fold_artifacts = []
    for index, validation_ids in enumerate(folds(reserve), start=1):
        train_ids = all_ids - validation_ids
        fold_dir = args.output_dir / f"fold-{index:02d}"
        fold_dir.mkdir(parents=True, exist_ok=True)
        train_pairs = fold_dir / "train-pairs.json"
        validation_pairs = fold_dir / "validation-pairs.json"
        shared.atomic_write(train_pairs, pair_subset(pairs, train_ids))
        shared.atomic_write(validation_pairs, pair_subset(pairs, validation_ids))
        candidate = fold_dir / "candidate.bin"
        if not candidate.exists():
            run_training(args.trainer, train_pairs, args.initial_weights, candidate, recipe, args.timeout)
        training = candidate.with_suffix(".ranking.json")
        metadata = shared.read(training)
        require(metadata["ranking_objective"] == recipe["objective"], f"fold {index}: objective drift")
        require(metadata["epochs"] == recipe["epochs"], f"fold {index}: epoch drift")
        candidate_audit = audit(args.ranking_auditor, validation_pairs, candidate)
        audit_path = fold_dir / "validation-audit.json"
        shared.atomic_write(audit_path, candidate_audit)
        validation_rows.extend(candidate_audit["model_diagnostic"]["parent_diagnostics"])
        fold_artifacts.append(
            {
                "fold": index,
                "validation_parent_ids": sorted(validation_ids),
                "train_pairs": shared.bind(train_pairs),
                "validation_pairs": shared.bind(validation_pairs),
                "candidate": shared.bind(candidate),
                "training": shared.bind(training),
                "validation_audit": shared.bind(audit_path),
            }
        )
    require({row["parent_id"] for row in validation_rows} == all_ids, "cross-validation parent coverage mismatch")
    candidate_metrics = metrics(validation_rows)
    reduction = (
        (baseline_metrics["mean_direct_top_regret_cp"] - candidate_metrics["mean_direct_top_regret_cp"])
        / baseline_metrics["mean_direct_top_regret_cp"]
    )
    conditions = {
        "mean_regret_reduction_at_least_10pct": reduction >= 0.10,
        "top1_not_decreased": candidate_metrics["top1_matches"] >= baseline_metrics["top1_matches"],
        "major_regrets_not_increased": candidate_metrics["major_regrets_ge_300cp"] <= baseline_metrics["major_regrets_ge_300cp"],
    }
    decision = {
        "schema": "sekirei.q25f-external-label-cv-decision.v1",
        "status": "transfer_signal_present" if all(conditions.values()) else "no_parent_level_transfer",
        "diagnostic_only": True,
        "strength_claim": False,
        "candidate_adopted": False,
        "q27_authorized": False,
        "q20_authorized": False,
        "q25e_holdout_read": False,
        "preregistration": shared.bind(prereg_path),
        "baseline": baseline_metrics,
        "candidate": candidate_metrics,
        "mean_regret_reduction": reduction,
        "conditions": conditions,
        "folds": fold_artifacts,
        "next_action": (
            "diagnose a single new external-label factor using a fresh score-blind reserve"
            if all(conditions.values())
            else "reject Q25e's external-label update as non-transferring; do not tune its recipe on these folds"
        ),
    }
    shared.atomic_write(args.output_dir / "decision.json", decision)
    return decision


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--family-preregistration", type=Path, required=True)
    parser.add_argument("--train-reserve", type=Path, required=True)
    parser.add_argument("--train-pairs", type=Path, required=True)
    parser.add_argument("--initial-weights", type=Path, required=True)
    parser.add_argument("--trainer", type=Path, required=True)
    parser.add_argument("--ranking-auditor", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=900.0)
    args = parser.parse_args()
    try:
        decision = run(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError, RuntimeError) as error:
        parser.error(str(error))
    print(json.dumps({key: decision[key] for key in ("status", "baseline", "candidate", "conditions")}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
