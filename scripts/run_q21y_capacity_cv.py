#!/usr/bin/env python3
"""Run Q21y's stratified capacity-only cross-validation.

The Q21x fresh hold-out is never read.  Every informative Q21w train parent is
held out exactly once in an eight-fold, stratum-preserving split.  Architecture
is the only changed factor; labels, optimizer, epochs, seed, objective, and
features stay fixed.
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
        if key in {
            "schema",
            "diagnostic_only",
            "strength_claim",
            "source_contract",
            "source_teacher",
            "pair_selection",
        }
    } | {"pairs": [row for row in document["pairs"] if row["parent_id"] in parent_ids]}


def folds(corpus: dict[str, Any], count: int = 8) -> list[set[str]]:
    by_category: dict[str, list[str]] = {}
    for row in corpus["positions"]:
        by_category.setdefault(row["category"], []).append(row["id"])
    require(all(len(values) == count for values in by_category.values()), "Q21y requires eight parents per stratum")
    result = [set() for _ in range(count)]
    for values in by_category.values():
        for index, identifier in enumerate(sorted(values)):
            result[index].add(identifier)
    return result


def audit(binary: Path, pairs: Path, weights: Path) -> dict[str, Any]:
    return shared.run_json([str(binary), "--pairs", str(pairs), "--weights", str(weights)], 120.0)


def metrics_from_rows(rows: list[dict[str, Any]]) -> dict[str, Any]:
    require(rows, "capacity CV produced no held-out parent diagnostics")
    return {
        "parents": len(rows),
        "mean_direct_top_regret_cp": mean(row["teacher_rank_loss_cp"] for row in rows),
        "top1_matches": sum(row["teacher_rank_loss_cp"] == 0 for row in rows),
        "major_regrets_ge_300cp": sum(row["teacher_rank_loss_cp"] >= 300 for row in rows),
    }


def score_signature(report: dict[str, Any]) -> list[tuple[str, str, int]]:
    return sorted(
        (row["parent_id"], row["move_usi"], row["parent_score_cp"])
        for row in report["model_diagnostic"]["move_diagnostics"]
    )


def run_training(
    trainer: Path,
    pairs: Path,
    initial: Path,
    output: Path,
    contract: dict[str, Any],
    timeout: float,
) -> None:
    command = [
        str(trainer),
        "--ranking-pairs", str(pairs),
        "--ranking-objective", contract["objective"],
        "--ranking-temperature-cp", str(contract["temperature_cp"]),
        "--ranking-parent-balanced",
        "--ranking-batch-pairs", str(contract["ranking_batch_pairs"]),
        "--ranking-epochs", str(contract["epochs"]),
        "--init-weights", str(initial),
        "--nnue-output", contract["nnue_output"],
        "--lr", str(contract["learning_rate"]),
        "--seed", str(contract["seed"]),
        "--output", str(output),
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
        raise RuntimeError(f"capacity training failed: {completed.stderr[-4000:]}")


def architecture(name: str, trainer: Path, auditor: Path, initial: Path, l1: int, l2: int) -> dict[str, Any]:
    return {
        "name": name,
        "dimensions": {"input": 2420, "l1": l1, "l2": l2},
        "features": "v0.3.42 default",
        "trainer": shared.bind(trainer),
        "auditor": shared.bind(auditor),
        "initial": shared.bind(initial),
    }


def run(args: argparse.Namespace) -> dict[str, Any]:
    q21x = shared.read(args.q21x_preregistration)
    pairs = shared.read(args.train_pairs)
    pair_audit = shared.read(args.pair_audit)
    corpus = shared.read(args.train_corpus)
    require(q21x.get("schema") == "sekirei.q21x-train-preregistration.v1", "unexpected Q21x preregistration")
    require(q21x["inputs"]["corpus"]["sha256"] == shared.sha256(args.train_corpus), "train corpus SHA mismatch")
    require(pairs.get("pair_selection") == "top-vs-rest", "Q21y requires direct top-vs-rest pairs")
    require(pair_audit.get("parents") == 72 and pair_audit.get("parents_with_pairs") > 0, "pair audit incomplete")
    architectures = [
        architecture("base-l1-256-l2-32", args.base_trainer, args.base_auditor, args.base_initial, 256, 32),
        architecture("l1-384-l2-32", args.l1_trainer, args.l1_auditor, args.l1_initial, 384, 32),
        architecture("l1-256-l2-64", args.l2_trainer, args.l2_auditor, args.l2_initial, 256, 64),
    ]
    prereg = {
        "schema": "sekirei.q21y-capacity-cv-preregistration.v1",
        "status": "frozen_before_capacity_training",
        "diagnostic_only": True,
        "strength_claim": False,
        "single_factor": "L1 or L2 width only; never both in one variant",
        "folds": 8,
        "selection_data": "Q21w train parents only; Q21x fresh holdout is retired and unread",
        "inputs": {
            "q21x_preregistration": shared.bind(args.q21x_preregistration),
            "train_corpus": shared.bind(args.train_corpus),
            "train_pairs": shared.bind(args.train_pairs),
            "pair_audit": shared.bind(args.pair_audit),
        },
        "training_contract": q21x["training_contract"],
        "architectures": architectures,
        "decision_contract": {
            "mean_direct_top_regret_reduction_minimum": 0.10,
            "top1_matches_must_not_decrease": True,
            "major_regrets_must_not_increase": True,
            "winner_limit": 1,
            "fresh_holdout_required_after_inner_cv": True,
        },
        "runner": shared.bind(Path(__file__).resolve()),
    }
    args.output_dir.mkdir(parents=True, exist_ok=True)
    prereg_path = args.output_dir / "preregistration.json"
    shared.atomic_write(prereg_path, prereg)
    all_parent_ids = {row["parent_id"] for row in pairs["pairs"]}
    fold_ids = folds(corpus)
    base_full = audit(args.base_auditor, args.train_pairs, args.base_initial)
    rows = []
    for spec, trainer, auditor, initial in (
        (architectures[0], args.base_trainer, args.base_auditor, args.base_initial),
        (architectures[1], args.l1_trainer, args.l1_auditor, args.l1_initial),
        (architectures[2], args.l2_trainer, args.l2_auditor, args.l2_initial),
    ):
        arm_dir = args.output_dir / spec["name"]
        arm_dir.mkdir(parents=True, exist_ok=True)
        initial_audit = audit(auditor, args.train_pairs, initial)
        shared.atomic_write(arm_dir / "initial-audit.json", initial_audit)
        output_preserved = score_signature(initial_audit) == score_signature(base_full)
        require(output_preserved, f"{spec['name']}: projected initial output differs from base")
        heldout_rows = []
        fold_artifacts = []
        for index, reserved_ids in enumerate(fold_ids):
            validation_ids = reserved_ids & all_parent_ids
            train_ids = all_parent_ids - validation_ids
            require(validation_ids and train_ids, f"{spec['name']}/fold {index}: empty split")
            fold_dir = arm_dir / f"fold-{index + 1:02d}"
            fold_dir.mkdir(parents=True, exist_ok=True)
            train_path = fold_dir / "train-pairs.json"
            validation_path = fold_dir / "validation-pairs.json"
            shared.atomic_write(train_path, pair_subset(pairs, train_ids))
            shared.atomic_write(validation_path, pair_subset(pairs, validation_ids))
            candidate = fold_dir / "candidate.bin"
            if not candidate.exists():
                run_training(trainer, train_path, initial, candidate, q21x["training_contract"], args.timeout)
            report = audit(auditor, validation_path, candidate)
            report_path = fold_dir / "validation-audit.json"
            shared.atomic_write(report_path, report)
            heldout_rows.extend(report["model_diagnostic"]["parent_diagnostics"])
            fold_artifacts.append(
                {
                    "fold": index + 1,
                    "heldout_parent_ids": sorted(validation_ids),
                    "train_pairs": shared.bind(train_path),
                    "validation_pairs": shared.bind(validation_path),
                    "candidate": shared.bind(candidate),
                    "validation_audit": shared.bind(report_path),
                }
            )
        require(len({row["parent_id"] for row in heldout_rows}) == len(all_parent_ids), f"{spec['name']}: parent coverage mismatch")
        rows.append(
            {
                "architecture": spec,
                "output_preserved_at_initialization": output_preserved,
                "metrics": metrics_from_rows(heldout_rows),
                "folds": fold_artifacts,
            }
        )
    baseline = rows[0]["metrics"]
    eligible = []
    for row in rows[1:]:
        current = row["metrics"]
        reduction = (
            (baseline["mean_direct_top_regret_cp"] - current["mean_direct_top_regret_cp"])
            / baseline["mean_direct_top_regret_cp"]
            if baseline["mean_direct_top_regret_cp"] > 0
            else 0.0
        )
        conditions = {
            "mean_regret_reduction_at_least_10pct": reduction >= 0.10,
            "top1_not_decreased": current["top1_matches"] >= baseline["top1_matches"],
            "major_regrets_not_increased": current["major_regrets_ge_300cp"] <= baseline["major_regrets_ge_300cp"],
        }
        row["comparison_to_base"] = {"mean_regret_reduction": reduction, "conditions": conditions, "pass": all(conditions.values())}
        if row["comparison_to_base"]["pass"]:
            eligible.append(row)
    winner = min(eligible, key=lambda row: row["metrics"]["mean_direct_top_regret_cp"]) if eligible else None
    decision = {
        "schema": "sekirei.q21y-capacity-cv-decision.v1",
        "status": "capacity_winner_frozen" if winner else "no_capacity_transfer",
        "diagnostic_only": True,
        "strength_claim": False,
        "q21x_holdout_reused": False,
        "q20_authorized": False,
        "preregistration": shared.bind(prereg_path),
        "rows": rows,
        "winner": winner["architecture"] if winner else None,
        "fresh_holdout_authorized": winner is not None,
        "next_action": (
            "reserve a new score-blind holdout for the one capacity winner"
            if winner
            else "close Q21y without a capacity winner; keep base capacity for Q21z"
        ),
    }
    shared.atomic_write(args.output_dir / "decision.json", decision)
    return decision


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--q21x-preregistration", type=Path, required=True)
    parser.add_argument("--train-corpus", type=Path, required=True)
    parser.add_argument("--train-pairs", type=Path, required=True)
    parser.add_argument("--pair-audit", type=Path, required=True)
    parser.add_argument("--base-trainer", type=Path, required=True)
    parser.add_argument("--base-auditor", type=Path, required=True)
    parser.add_argument("--base-initial", type=Path, required=True)
    parser.add_argument("--l1-trainer", type=Path, required=True)
    parser.add_argument("--l1-auditor", type=Path, required=True)
    parser.add_argument("--l1-initial", type=Path, required=True)
    parser.add_argument("--l2-trainer", type=Path, required=True)
    parser.add_argument("--l2-auditor", type=Path, required=True)
    parser.add_argument("--l2-initial", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=900.0)
    args = parser.parse_args()
    try:
        decision = run(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError, RuntimeError) as error:
        parser.error(str(error))
    print(json.dumps({"status": decision["status"], "winner": decision["winner"], "metrics": {row["architecture"]["name"]: row["metrics"] for row in decision["rows"]}}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
