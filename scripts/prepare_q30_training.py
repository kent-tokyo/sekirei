#!/usr/bin/env python3
"""Freeze Q30's shared-label runtime for 256/32 versus 128/16 NNUE."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

import prepare_q28_training as common


RUNTIME_SCHEMA = "sekirei.q30-runtime-preregistration.v1"
LABEL_SCHEMA = "sekirei.q30-train-preregistration.v1"
FAMILY_SCHEMA = "sekirei.q30-efficiency-family-preregistration.v1"


def require_positions(document: dict[str, Any], schema: str, count: int) -> list[dict[str, Any]]:
    common.require(document.get("schema") == schema, f"unexpected {schema}")
    rows = document.get("positions")
    common.require(isinstance(rows, list) and len(rows) == count, f"expected {count} positions")
    return rows


def verify_disjoint(train: list[dict[str, Any]], holdout: list[dict[str, Any]]) -> None:
    for field in ("source_key", "derived_group"):
        left = {row.get("source", {}).get(field) for row in train}
        right = {row.get("source", {}).get(field) for row in holdout}
        common.require(None not in left | right, f"missing {field}")
        common.require(len(left) == len(train) and len(right) == len(holdout), f"non-unique {field}")
        common.require(not (left & right), f"train/holdout {field} overlap")


def runtime(args: argparse.Namespace) -> dict[str, Any]:
    family = common.read(args.family)
    train_reserve = common.read(args.train_reserve)
    holdout_reserve = common.read(args.holdout_reserve)
    common.require(
        family.get("schema") == FAMILY_SCHEMA and family.get("status") == "frozen_before_any_teacher_label",
        "unexpected Q30 family",
    )
    train = require_positions(train_reserve, "sekirei.q30-train-reserve.v1", 72)
    holdout = require_positions(holdout_reserve, "sekirei.q30-holdout-reserve.v1", 18)
    verify_disjoint(train, holdout)
    fixed = {
        "output": "residual-material", "epochs": 60, "learning_rate": 0.001,
        "seed": 42, "objective": "listwise-softmax", "temperature_cp": 400,
    }
    return {
        "schema": RUNTIME_SCHEMA,
        "status": "frozen_before_shallow_labels",
        "diagnostic_only": True,
        "strength_claim": False,
        "single_factor": family["single_factor"],
        "parents": 72,
        "strata": sorted({row["category"] for row in train}),
        "inputs": {
            "family": common.bind(args.family),
            "corpus": common.bind(args.train_reserve),
            "holdout_reserve_sealed": common.bind(args.holdout_reserve),
            # The generic shallow/depth-7 runners bind the evaluator using
            # these established keys.  Keep teacher-prefixed aliases too so
            # the Q30 manifest makes the role explicit without changing the
            # runner contract.
            "engine": common.bind(args.teacher_engine),
            "weights": common.bind(args.teacher_weights),
            "teacher_engine": common.bind(args.teacher_engine),
            "teacher_weights": common.bind(args.teacher_weights),
            "base_trainer": common.bind(args.base_trainer),
            "base_ranking_auditor": common.bind(args.base_ranking_auditor),
            "base_initial_weights": common.bind(args.base_initial_weights),
            "reduced_trainer": common.bind(args.reduced_trainer),
            "reduced_ranking_auditor": common.bind(args.reduced_ranking_auditor),
            "reduced_initial_weights": common.bind(args.reduced_initial_weights),
            "reduced_projection_metadata": common.bind(args.reduced_projection_metadata),
        },
        "shallow_contract": {
            "max_depth": 3, "root_candidates": 600, "threads": 1, "spec_top_n": 0,
            "timeout_seconds_per_search": 600, "nnue_output": fixed["output"],
        },
        "teacher_contract": {
            "max_depth": 7, "threads": 1, "spec_top_n": 0, "timeout_seconds_per_search": 600,
            "nnue_output": fixed["output"], "binary_sha256": common.sha256(args.teacher_engine),
            "weights_sha256": common.sha256(args.teacher_weights),
        },
        "candidate_contract": {
            "mode": "preregistered_candidate_union",
            "members": "depth-3 complete-root top 8 union every repeated depth-7 free bestmove",
            "free_repeats": 2, "fixed_root_repeats": 2, "maximum_moves_per_parent": 10,
            "top_k_after_depth7_reranking": 8, "normal_score_abs_max_cp": 10000,
            "pair_selection": "top-vs-rest", "require_aa_signature_match": True,
            "require_exact_completed_search": True,
        },
        "training_contract": {
            "objective": fixed["objective"], "temperature_cp": fixed["temperature_cp"],
            "epochs": fixed["epochs"], "learning_rate": fixed["learning_rate"],
            "fresh_optimizer": True, "seed": fixed["seed"], "ranking_batch_pairs": 1,
            "ranking_parent_balanced": True, "nnue_output": fixed["output"],
            "features": "v0.3.42 default; king_relative_b_small disabled",
            "architectures": {
                "base": {"input": 2420, "l1": 256, "l2": 32, "initialization": "fixed checkpoint"},
                "reduced": {"input": 2420, "l1": 128, "l2": 16, "initialization": "deterministic prefix subnetwork; output non-preserving"},
            },
        },
        "screen_contract": {
            "static_max_relative_regret_worsening_vs_base": 0.10,
            "reduced_major_regrets_must_not_exceed_base": True,
            "same_time_reduced_must_not_trail_material": True,
            "time_ms": 1000,
            "repeats_per_arm": 2,
            "q27_requires": "all static and same-time requirements",
        },
        "tools": {
            "runtime_preparer": common.bind(Path(__file__).resolve()),
            "shallow_runner": common.bind(args.shallow_runner),
            "label_runner": common.bind(args.label_runner),
            "pair_builder": common.bind(args.pair_builder),
            "train_runner": common.bind(args.train_runner),
        },
    }


def labels(args: argparse.Namespace) -> dict[str, Any]:
    runtime_document = common.read(args.runtime_preregistration)
    shallow = common.read(args.shallow_teacher)
    common.require(
        runtime_document.get("schema") == RUNTIME_SCHEMA
        and runtime_document.get("status") == "frozen_before_shallow_labels",
        "unexpected Q30 runtime",
    )
    common.require(
        shallow.get("schema") == "sekirei.root-rank-teacher-corpus.v1"
        and len(shallow.get("rows", [])) == runtime_document["parents"],
        "incomplete Q30 shallow labels",
    )
    common.require(
        shallow.get("source_corpus", {}).get("sha256") == runtime_document["inputs"]["corpus"]["sha256"],
        "Q30 shallow corpus mismatch",
    )
    return {
        **runtime_document,
        "schema": LABEL_SCHEMA,
        "status": "frozen_before_train_labels",
        "inputs": {**runtime_document["inputs"], "shallow_teacher": common.bind(args.shallow_teacher)},
        "runtime_preregistration": common.bind(args.runtime_preregistration),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    runtime_parser = commands.add_parser("runtime")
    for name in (
        "family", "train-reserve", "holdout-reserve", "teacher-engine", "teacher-weights",
        "base-trainer", "base-ranking-auditor", "base-initial-weights", "reduced-trainer",
        "reduced-ranking-auditor", "reduced-initial-weights", "reduced-projection-metadata",
        "shallow-runner", "label-runner", "pair-builder", "train-runner", "output",
    ):
        runtime_parser.add_argument(f"--{name}", type=Path, required=True)
    label_parser = commands.add_parser("labels")
    label_parser.add_argument("--runtime-preregistration", type=Path, required=True)
    label_parser.add_argument("--shallow-teacher", type=Path, required=True)
    label_parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        document = runtime(args) if args.command == "runtime" else labels(args)
        common.write_immutable(args.output, document)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    print(json.dumps({"schema": document["schema"], "status": document["status"]}, sort_keys=True))


if __name__ == "__main__":
    main()
