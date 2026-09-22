#!/usr/bin/env python3
"""Run Q21z's one-factor king-relative feature cross-validation."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

import run_q21u_train_screen as shared
import run_q21y_capacity_cv as q21y


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def run(args: argparse.Namespace) -> dict[str, Any]:
    q21x = shared.read(args.q21x_preregistration)
    q21y_decision = shared.read(args.q21y_decision)
    pairs = shared.read(args.train_pairs)
    corpus = shared.read(args.train_corpus)
    require(q21x.get("schema") == "sekirei.q21x-train-preregistration.v1", "unexpected Q21x preregistration")
    require(
        q21y_decision.get("schema") == "sekirei.q21y-capacity-cv-decision.v1"
        and q21y_decision.get("status") == "no_capacity_transfer"
        and q21y_decision.get("winner") is None,
        "Q21z expects Q21y to retain base capacity",
    )
    require(pairs.get("pair_selection") == "top-vs-rest", "Q21z requires direct top-vs-rest pairs")
    base_row = next(row for row in q21y_decision["rows"] if row["architecture"]["name"] == "base-l1-256-l2-32")
    feature = {
        "name": "king-relative-b-small",
        "capacity": {"l1": 256, "l2": 32},
        "feature_change": "partition board features by own-king 3x3 zone; hand features unchanged",
        "trainer": shared.bind(args.feature_trainer),
        "auditor": shared.bind(args.feature_auditor),
        "initial": shared.bind(args.feature_initial),
    }
    prereg = {
        "schema": "sekirei.q21z-feature-cv-preregistration.v1",
        "status": "frozen_before_feature_training",
        "diagnostic_only": True,
        "strength_claim": False,
        "single_factor": "king-relative board-feature partition only; capacity remains L1=256/L2=32",
        "folds": 8,
        "selection_data": "Q21w train parents only; no Q21x holdout reuse",
        "inputs": {
            "q21x_preregistration": shared.bind(args.q21x_preregistration),
            "q21y_decision": shared.bind(args.q21y_decision),
            "train_corpus": shared.bind(args.train_corpus),
            "train_pairs": shared.bind(args.train_pairs),
            "pair_audit": shared.bind(args.pair_audit),
            "base_initial": shared.bind(args.base_initial),
        },
        "training_contract": q21x["training_contract"],
        "feature": feature,
        "decision_contract": {
            "mean_direct_top_regret_reduction_minimum": 0.10,
            "top1_matches_must_not_decrease": True,
            "major_regrets_must_not_increase": True,
            "fresh_holdout_required_after_inner_cv": True,
        },
        "runner": shared.bind(Path(__file__).resolve()),
    }
    args.output_dir.mkdir(parents=True, exist_ok=True)
    prereg_path = args.output_dir / "preregistration.json"
    shared.atomic_write(prereg_path, prereg)
    base_full = q21y.audit(args.base_auditor, args.train_pairs, args.base_initial)
    feature_initial_audit = q21y.audit(args.feature_auditor, args.train_pairs, args.feature_initial)
    initial_audit_path = args.output_dir / "initial-audit.json"
    shared.atomic_write(initial_audit_path, feature_initial_audit)
    output_preserved = q21y.score_signature(base_full) == q21y.score_signature(feature_initial_audit)
    require(output_preserved, "king-relative projection does not preserve initial scores")
    all_parent_ids = {row["parent_id"] for row in pairs["pairs"]}
    heldout_rows = []
    fold_artifacts = []
    for index, reserved_ids in enumerate(q21y.folds(corpus)):
        validation_ids = reserved_ids & all_parent_ids
        train_ids = all_parent_ids - validation_ids
        require(validation_ids and train_ids, f"fold {index + 1}: empty split")
        fold_dir = args.output_dir / f"fold-{index + 1:02d}"
        fold_dir.mkdir(parents=True, exist_ok=True)
        train_path = fold_dir / "train-pairs.json"
        validation_path = fold_dir / "validation-pairs.json"
        shared.atomic_write(train_path, q21y.pair_subset(pairs, train_ids))
        shared.atomic_write(validation_path, q21y.pair_subset(pairs, validation_ids))
        candidate = fold_dir / "candidate.bin"
        if not candidate.exists():
            q21y.run_training(
                args.feature_trainer,
                train_path,
                args.feature_initial,
                candidate,
                q21x["training_contract"],
                args.timeout,
            )
        report = q21y.audit(args.feature_auditor, validation_path, candidate)
        report_path = fold_dir / "validation-audit.json"
        shared.atomic_write(report_path, report)
        heldout_rows.extend(report["model_diagnostic"]["parent_diagnostics"])
        fold_artifacts.append(
            {
                "fold": index + 1,
                "heldout_parent_ids": sorted(validation_ids),
                "candidate": shared.bind(candidate),
                "validation_audit": shared.bind(report_path),
            }
        )
    require(len({row["parent_id"] for row in heldout_rows}) == len(all_parent_ids), "Q21z parent coverage mismatch")
    metrics = q21y.metrics_from_rows(heldout_rows)
    baseline = base_row["metrics"]
    reduction = (
        (baseline["mean_direct_top_regret_cp"] - metrics["mean_direct_top_regret_cp"])
        / baseline["mean_direct_top_regret_cp"]
        if baseline["mean_direct_top_regret_cp"] > 0
        else 0.0
    )
    conditions = {
        "mean_regret_reduction_at_least_10pct": reduction >= 0.10,
        "top1_not_decreased": metrics["top1_matches"] >= baseline["top1_matches"],
        "major_regrets_not_increased": metrics["major_regrets_ge_300cp"] <= baseline["major_regrets_ge_300cp"],
    }
    passed = all(conditions.values())
    decision = {
        "schema": "sekirei.q21z-feature-cv-decision.v1",
        "status": "feature_winner_frozen" if passed else "no_feature_transfer",
        "diagnostic_only": True,
        "strength_claim": False,
        "q21x_holdout_reused": False,
        "q20_authorized": False,
        "preregistration": shared.bind(prereg_path),
        "output_preserved_at_initialization": output_preserved,
        "baseline_metrics": baseline,
        "feature_metrics": metrics,
        "mean_regret_reduction": reduction,
        "conditions": conditions,
        "pass": passed,
        "winner": feature if passed else None,
        "fresh_holdout_authorized": passed,
        "folds": fold_artifacts,
        "next_action": (
            "reserve a new score-blind holdout for the one feature winner"
            if passed
            else "close Q21z without a feature winner; do not combine failed factors"
        ),
    }
    shared.atomic_write(args.output_dir / "decision.json", decision)
    return decision


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--q21x-preregistration", type=Path, required=True)
    parser.add_argument("--q21y-decision", type=Path, required=True)
    parser.add_argument("--train-corpus", type=Path, required=True)
    parser.add_argument("--train-pairs", type=Path, required=True)
    parser.add_argument("--pair-audit", type=Path, required=True)
    parser.add_argument("--base-auditor", type=Path, required=True)
    parser.add_argument("--base-initial", type=Path, required=True)
    parser.add_argument("--feature-trainer", type=Path, required=True)
    parser.add_argument("--feature-auditor", type=Path, required=True)
    parser.add_argument("--feature-initial", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=1800.0)
    args = parser.parse_args()
    try:
        decision = run(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError, RuntimeError) as error:
        parser.error(str(error))
    print(json.dumps({"status": decision["status"], "baseline": decision["baseline_metrics"], "feature": decision["feature_metrics"], "reduction": decision["mean_regret_reduction"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
