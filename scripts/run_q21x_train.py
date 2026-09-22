#!/usr/bin/env python3
"""Train and freeze Q21x's single preregistered baseline candidate."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
from pathlib import Path
from typing import Any

import run_q21u_train_screen as q21u


SCHEMA = "sekirei.q21x-train-preregistration.v1"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def run(args: argparse.Namespace) -> dict[str, Any]:
    prereg = q21u.read(args.preregistration)
    pairs = q21u.read(args.train_pairs)
    pair_audit = q21u.read(args.pair_audit)
    require(
        prereg.get("schema") == SCHEMA and prereg.get("status") == "frozen_before_train_labels",
        "unexpected Q21x preregistration",
    )
    for name, path in (
        ("weights", args.initial_weights),
        ("trainer", args.trainer),
        ("ranking_auditor", args.ranking_auditor),
    ):
        require(prereg["inputs"][name]["sha256"] == q21u.sha256(path), f"{name} SHA mismatch")
    require(
        pairs.get("schema") == "sekirei.root-rank-pairs.v1"
        and pairs.get("pair_selection") == "top-vs-rest",
        "Q21x requires direct top-versus-rest pairs",
    )
    parent_ids = {row["parent_id"] for row in pairs.get("pairs", [])}
    coverage = {
        "total_labeled": pair_audit.get("parents"),
        "with_strict_preference": pair_audit.get("parents_with_pairs"),
        "excluded_as_noninformative": pair_audit.get("parents_excluded"),
    }
    require(
        coverage.get("total_labeled") == prereg["parents"]
        and coverage.get("with_strict_preference") == len(parent_ids)
        and coverage.get("excluded_as_noninformative") == prereg["parents"] - len(parent_ids),
        "Q21x pair coverage does not account for every frozen train parent",
    )
    categories = {row["category"] for row in pairs.get("pairs", [])}
    corpus_path = Path(prereg["inputs"]["corpus"]["path"])
    require(q21u.sha256(corpus_path) == prereg["inputs"]["corpus"]["sha256"], "Q21x corpus SHA mismatch")
    frozen_categories = {row["category"] for row in q21u.read(corpus_path)["positions"]}
    require(categories == frozen_categories, "Q21x informative pairs do not cover every frozen stratum")
    contract = prereg["training_contract"]
    args.output_dir.mkdir(parents=True, exist_ok=True)
    baseline_audit = q21u.audit(args.ranking_auditor, args.train_pairs, args.initial_weights)
    q21u.atomic_write(args.output_dir / "train-baseline-audit.json", baseline_audit)
    baseline = q21u.metrics(baseline_audit)
    candidate = args.output_dir / "candidate.bin"
    metadata = candidate.with_suffix(".ranking.json")
    if not candidate.exists():
        command = [
            str(args.trainer),
            "--ranking-pairs", str(args.train_pairs),
            "--ranking-objective", contract["objective"],
            "--ranking-temperature-cp", str(contract["temperature_cp"]),
            "--ranking-parent-balanced",
            "--ranking-batch-pairs", str(contract["ranking_batch_pairs"]),
            "--ranking-epochs", str(contract["epochs"]),
            "--init-weights", str(args.initial_weights),
            "--nnue-output", contract["nnue_output"],
            "--lr", str(contract["learning_rate"]),
            "--seed", str(contract["seed"]),
            "--output", str(candidate),
        ]
        completed = subprocess.run(
            command,
            text=True,
            capture_output=True,
            check=False,
            timeout=args.timeout,
            env={**os.environ, "RAYON_NUM_THREADS": "1"},
        )
        if completed.returncode:
            raise RuntimeError(f"Q21x training failed: {completed.stderr[-4000:]}")
    training = q21u.read(metadata)
    require(
        training.get("ranking_objective") == contract["objective"]
        and training.get("ranking_temperature_cp") == contract["temperature_cp"]
        and training.get("epochs") == contract["epochs"],
        "Q21x training metadata mismatch",
    )
    candidate_audit = q21u.audit(args.ranking_auditor, args.train_pairs, candidate)
    q21u.atomic_write(args.output_dir / "train-candidate-audit.json", candidate_audit)
    candidate_metrics = q21u.metrics(candidate_audit)
    progress = (
        training.get("parameters_changed", 0) > 0
        and candidate_metrics["major_regrets_ge_300cp"] <= baseline["major_regrets_ge_300cp"]
        and (
            candidate_metrics["top1_matches"] > baseline["top1_matches"]
            or candidate_metrics["mean_direct_top_regret_cp"]
            < baseline["mean_direct_top_regret_cp"]
        )
    )
    decision = {
        "schema": "sekirei.q21x-train-decision.v1",
        "status": "candidate_frozen" if progress else "no_train_progress",
        "diagnostic_only": True,
        "strength_claim": False,
        "holdout_inspected": False,
        "holdout_labels_generated": False,
        "candidate_adopted": False,
        "q20_authorized": False,
        "preregistration": q21u.bind(args.preregistration),
        "train_pairs": q21u.bind(args.train_pairs),
        "pair_audit": q21u.bind(args.pair_audit),
        "baseline": baseline,
        "candidate": {"artifact": q21u.bind(candidate), "metrics": candidate_metrics},
        "training": q21u.bind(metadata),
        "minimum_train_progress_pass": progress,
        "tool_provenance": {
            "preregistered_sha256": prereg["tools"]["train_runner"]["sha256"],
            "executed_sha256": q21u.sha256(Path(__file__).resolve()),
            "contract_preserving_fix_after_preregistration": (
                prereg["tools"]["train_runner"]["sha256"]
                != q21u.sha256(Path(__file__).resolve())
            ),
            "fix_scope": (
                "accept fully labeled parents with no strict ordinary-cp preference as "
                "noninformative while requiring every frozen parent to be accounted for and "
                "all nine strata to retain informative pairs"
            ),
        },
        "next_action": (
            "freeze Q21x holdout runtime before generating any holdout label"
            if progress
            else "stop Q21x without opening holdout; continue to preregistered capacity diagnosis"
        ),
        "runner": q21u.bind(Path(__file__).resolve()),
    }
    q21u.atomic_write(args.output_dir / "train-decision.json", decision)
    return decision


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preregistration", type=Path, required=True)
    parser.add_argument("--train-pairs", type=Path, required=True)
    parser.add_argument("--pair-audit", type=Path, required=True)
    parser.add_argument("--initial-weights", type=Path, required=True)
    parser.add_argument("--trainer", type=Path, required=True)
    parser.add_argument("--ranking-auditor", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=3600.0)
    args = parser.parse_args()
    try:
        decision = run(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError, RuntimeError) as error:
        parser.error(str(error))
    print(json.dumps({"status": decision["status"], "baseline": decision["baseline"], "candidate": decision["candidate"]["metrics"]}, sort_keys=True))
    return 0 if decision["minimum_train_progress_pass"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
