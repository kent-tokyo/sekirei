#!/usr/bin/env python3
"""Freeze Q21v's train-to-holdout transfer audit before LOPO training."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def bind(path: Path) -> dict[str, str]:
    if not path.is_file():
        raise ValueError(f"missing input: {path}")
    return {"path": str(path), "sha256": sha256(path)}


def read(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def prepare(args: argparse.Namespace) -> dict[str, Any]:
    recipe = read(args.recipe_decision)
    validation = read(args.validation_decision)
    train_pairs = read(args.train_pairs)
    validation_pairs = read(args.validation_pairs)
    require(
        recipe.get("schema") == "sekirei.q21u-listwise-train-recipe-decision.v1"
        and recipe.get("status") == "recipe_frozen"
        and recipe.get("selected", {}).get("candidate", {}).get("sha256")
        == sha256(args.candidate),
        "Q21u frozen recipe/candidate mismatch",
    )
    require(
        validation.get("schema") == "sekirei.q21u-listwise-validation-decision.v1"
        and validation.get("status") == "fail"
        and validation.get("experiment_complete") is True
        and validation.get("development_match_authorized") is False,
        "Q21u validation decision is not the completed rejection",
    )
    train_parents = sorted({row["parent_id"] for row in train_pairs.get("pairs", [])})
    validation_parents = sorted({row["parent_id"] for row in validation_pairs.get("pairs", [])})
    require(len(train_parents) == 18, "Q21v requires the frozen 18-parent Q21u train set")
    require(len(validation_parents) == 9, "Q21v requires the frozen 9-parent Q21u holdout")
    require(set(train_parents).isdisjoint(validation_parents), "train and validation parents overlap")
    selected_recipe = recipe["selected"]["recipe"]
    require(
        selected_recipe == {"id": "e60-lr0010", "epochs": 60, "learning_rate": 0.001},
        "unexpected Q21u selected recipe",
    )
    document = {
        "schema": "sekirei.q21v-transfer-audit-preregistration.v1",
        "status": "frozen_before_lopo_training",
        "diagnostic_only": True,
        "strength_claim": False,
        "candidate_adopted": False,
        "hypothesis": (
            "Q21u's full-train improvement failed to transfer because of parent memorization, "
            "train/holdout teacher-distribution shift, hidden-layer activation collapse, or a "
            "combination; architecture capacity is not changed in this audit"
        ),
        "validation_boundary": (
            "Q21u validation is audit-only and must not be used for training, recipe selection, "
            "temperature selection, or checkpoint selection"
        ),
        "contract": {
            "train_parents": len(train_parents),
            "validation_parents": len(validation_parents),
            "lopo_folds": len(train_parents),
            "lopo_heldout_parents_per_fold": 1,
            "ranking_objective": "listwise-softmax",
            "temperature_cp": recipe["temperature_cp"],
            "epochs": selected_recipe["epochs"],
            "learning_rate": selected_recipe["learning_rate"],
            "seed": 42,
            "fresh_optimizer_per_fold": True,
            "nnue_output": "residual-material",
            "threads": 1,
            "overfit_signal": (
                "full-train direct-regret reduction >=10% and LOPO reduction <5%"
            ),
            "distribution_shift_signal": (
                "absolute relative difference >=25% in mean candidate count or median nonzero "
                "teacher gap"
            ),
            "activation_collapse_signal": (
                "mean FT/L2 active ratio <10% or saturated ratio >50% in any frozen corpus/arm"
            ),
        },
        "inputs": {
            "recipe_decision": bind(args.recipe_decision),
            "validation_decision": bind(args.validation_decision),
            "train_pairs": bind(args.train_pairs),
            "validation_pairs": bind(args.validation_pairs),
            "initial_weights": bind(args.initial_weights),
            "candidate": bind(args.candidate),
            "trainer": bind(args.trainer),
            "ranking_auditor": bind(args.ranking_auditor),
            "preparer": bind(Path(__file__).resolve()),
            "runner": bind(args.runner),
        },
    }
    args.output_dir.mkdir(parents=True, exist_ok=True)
    output = args.output_dir / "preregistration.json"
    output.write_text(
        json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return document


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--recipe-decision", type=Path, required=True)
    parser.add_argument("--validation-decision", type=Path, required=True)
    parser.add_argument("--train-pairs", type=Path, required=True)
    parser.add_argument("--validation-pairs", type=Path, required=True)
    parser.add_argument("--initial-weights", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--trainer", type=Path, required=True)
    parser.add_argument("--ranking-auditor", type=Path, required=True)
    parser.add_argument("--runner", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    try:
        document = prepare(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    print(json.dumps({"status": document["status"], "lopo_folds": document["contract"]["lopo_folds"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
