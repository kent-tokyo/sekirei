#!/usr/bin/env python3
"""Run Q21u's preregistered train-only recipe screen and freeze one recipe."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
from pathlib import Path
from typing import Any


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def bind(path: Path) -> dict[str, str]:
    if not path.is_file():
        raise ValueError(f"missing artifact: {path}")
    return {"path": str(path), "sha256": sha256(path)}


def read(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def atomic_write(path: Path, document: dict[str, Any]) -> None:
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(
        json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    os.replace(temporary, path)


def run_json(command: list[str], timeout: float) -> dict[str, Any]:
    completed = subprocess.run(
        command,
        text=True,
        capture_output=True,
        check=False,
        timeout=timeout,
        env={**os.environ, "RAYON_NUM_THREADS": "1"},
    )
    if completed.returncode:
        raise RuntimeError(
            f"command failed ({completed.returncode}): {' '.join(command)}\n{completed.stderr[-4000:]}"
        )
    return json.loads(completed.stdout)


def metrics(audit: dict[str, Any]) -> dict[str, Any]:
    model = audit["model_diagnostic"]
    rows = model["parent_diagnostics"]
    return {
        "parents": len(rows),
        "mean_direct_top_regret_cp": model["mean_parent_rank_loss_cp"],
        "top1_matches": sum(row["teacher_rank_loss_cp"] == 0 for row in rows),
        "major_regrets_ge_300cp": sum(row["teacher_rank_loss_cp"] >= 300 for row in rows),
        "ordered_pair_rate": model["teacher_preferred_ordering_rate"],
    }


def audit(auditor: Path, pairs: Path, weights: Path) -> dict[str, Any]:
    return run_json(
        [str(auditor), "--pairs", str(pairs), "--weights", str(weights)],
        60.0,
    )


def run(args: argparse.Namespace) -> dict[str, Any]:
    prereg = read(args.preregistration)
    require(
        prereg.get("schema") == "sekirei.q21u-listwise-train-preregistration.v1"
        and prereg.get("status") == "frozen_before_any_q21u_training_or_validation_selection",
        "unexpected Q21u preregistration",
    )
    for name, path in (
        ("train_pairs", args.train_pairs),
        ("initial_weights", args.initial_weights),
        ("trainer", args.trainer),
        ("ranking_auditor", args.ranking_auditor),
    ):
        require(prereg["inputs"][name]["sha256"] == sha256(path), f"{name} SHA mismatch")
    temperature = prereg["temperature_calibration"]["selected_temperature_cp"]
    args.output_dir.mkdir(parents=True, exist_ok=True)
    baseline_audit = audit(args.ranking_auditor, args.train_pairs, args.initial_weights)
    atomic_write(args.output_dir / "train-baseline-audit.json", baseline_audit)
    baseline = metrics(baseline_audit)
    rows = []
    for recipe in prereg["recipe_search"]["recipes"]:
        recipe_dir = args.output_dir / recipe["id"]
        recipe_dir.mkdir(parents=True, exist_ok=True)
        candidate = recipe_dir / "candidate.bin"
        metadata = candidate.with_suffix(".ranking.json")
        if not candidate.exists():
            completed = subprocess.run(
                [
                    str(args.trainer),
                    "--ranking-pairs", str(args.train_pairs),
                    "--ranking-objective", "listwise-softmax",
                    "--ranking-temperature-cp", str(temperature),
                    "--ranking-parent-balanced",
                    "--ranking-batch-pairs", "1",
                    "--ranking-epochs", str(recipe["epochs"]),
                    "--init-weights", str(args.initial_weights),
                    "--nnue-output", "residual-material",
                    "--lr", str(recipe["learning_rate"]),
                    "--seed", str(prereg["recipe_search"]["seed"]),
                    "--output", str(candidate),
                ],
                text=True,
                capture_output=True,
                check=False,
                timeout=900,
                env={**os.environ, "RAYON_NUM_THREADS": "1"},
            )
            if completed.returncode:
                raise RuntimeError(f"{recipe['id']}: training failed: {completed.stderr[-4000:]}")
        training = read(metadata)
        require(
            training.get("ranking_objective") == "listwise-softmax"
            and training.get("ranking_temperature_cp") == temperature
            and training.get("epochs") == recipe["epochs"],
            f"{recipe['id']}: training metadata mismatch",
        )
        candidate_audit = audit(args.ranking_auditor, args.train_pairs, candidate)
        atomic_write(recipe_dir / "train-audit.json", candidate_audit)
        candidate_metrics = metrics(candidate_audit)
        rows.append(
            {
                "recipe": recipe,
                "candidate": bind(candidate),
                "training": bind(metadata),
                "audit": bind(recipe_dir / "train-audit.json"),
                "metrics": candidate_metrics,
                "final_listwise_cross_entropy": training["mean_listwise_cross_entropy_final"],
                "parameter_update_l2": training["parameter_update_l2"],
                "parameter_update_max_abs": training["parameter_update_max_abs"],
                "parameters_changed": training["parameters_changed"],
            }
        )
    selected = min(
        rows,
        key=lambda row: (
            row["metrics"]["major_regrets_ge_300cp"],
            -row["metrics"]["top1_matches"],
            row["metrics"]["mean_direct_top_regret_cp"],
            row["final_listwise_cross_entropy"],
            row["recipe"]["id"],
        ),
    )
    selected_metrics = selected["metrics"]
    minimum_progress = (
        selected["parameters_changed"] > 0
        and selected_metrics["major_regrets_ge_300cp"] <= baseline["major_regrets_ge_300cp"]
        and (
            selected_metrics["top1_matches"] > baseline["top1_matches"]
            or selected_metrics["mean_direct_top_regret_cp"]
            < baseline["mean_direct_top_regret_cp"]
        )
    )
    decision = {
        "schema": "sekirei.q21u-listwise-train-recipe-decision.v1",
        "status": "recipe_frozen" if minimum_progress else "no_train_progress",
        "diagnostic_only": True,
        "strength_claim": False,
        "validation_inspected": False,
        "preregistration": bind(args.preregistration),
        "baseline": baseline,
        "temperature_cp": temperature,
        "recipes": rows,
        "selected": selected,
        "minimum_progress_pass": minimum_progress,
        "next_action": (
            "freeze a fresh validation holdout before labels"
            if minimum_progress
            else "stop without selecting validation positions; redesign the listwise update"
        ),
        "runner": bind(Path(__file__).resolve()),
    }
    atomic_write(args.output_dir / "recipe-decision.json", decision)
    return decision


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preregistration", type=Path, required=True)
    parser.add_argument("--train-pairs", type=Path, required=True)
    parser.add_argument("--initial-weights", type=Path, required=True)
    parser.add_argument("--trainer", type=Path, required=True)
    parser.add_argument("--ranking-auditor", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    try:
        decision = run(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError, RuntimeError) as error:
        parser.error(str(error))
    print(json.dumps({
        "status": decision["status"],
        "selected": decision["selected"]["recipe"]["id"],
        "baseline": decision["baseline"],
        "candidate": decision["selected"]["metrics"],
    }, sort_keys=True))
    return 0 if decision["minimum_progress_pass"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
