#!/usr/bin/env python3
"""Freeze Q28 runtime, then freeze its depth-7 label contract."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any


RUNTIME_SCHEMA = "sekirei.q28-runtime-preregistration.v1"
LABEL_SCHEMA = "sekirei.q28-train-preregistration.v1"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def bind(path: Path) -> dict[str, str]:
    if not path.is_file():
        raise ValueError(f"missing input: {path}")
    return {"path": str(path), "sha256": sha256(path)}


def read(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def positions(document: dict[str, Any], schema: str, count: int) -> list[dict[str, Any]]:
    require(document.get("schema") == schema, f"unexpected {schema} input")
    rows = document.get("positions")
    require(isinstance(rows, list) and len(rows) == count, f"expected {count} positions")
    return rows


def validate_disjoint(train: list[dict[str, Any]], holdout: list[dict[str, Any]]) -> None:
    train_sources = {row.get("source", {}).get("source_key") for row in train}
    holdout_sources = {row.get("source", {}).get("source_key") for row in holdout}
    require(None not in train_sources | holdout_sources, "missing CSA source key")
    require(len(train_sources) == len(train) and len(holdout_sources) == len(holdout), "source uniqueness drift")
    require(not (train_sources & holdout_sources), "train/holdout source overlap")


def runtime(args: argparse.Namespace) -> dict[str, Any]:
    family = read(args.family)
    execution = read(args.execution)
    train = read(args.train_reserve)
    holdout = read(args.holdout_reserve)
    require(
        family.get("schema") == "sekirei.q28-tactical-source-family-preregistration.v1"
        and family.get("status") == "frozen_before_score_blind_parent_selection",
        "unexpected Q28 family",
    )
    require(
        execution.get("schema") == "sekirei.q28-execution-preregistration.v1"
        and execution.get("status") == "frozen_before_any_self_teacher_label",
        "unexpected Q28 execution boundary",
    )
    train_rows = positions(train, "sekirei.q28-train-reserve.v1", 72)
    holdout_rows = positions(holdout, "sekirei.q28-holdout-reserve.v1", 18)
    validate_disjoint(train_rows, holdout_rows)
    fixed = family["fixed_training_contract"]
    document = {
        "schema": RUNTIME_SCHEMA,
        "status": "frozen_before_shallow_labels",
        "diagnostic_only": True,
        "strength_claim": False,
        "single_factor": family["single_factor"],
        "parents": 72,
        "strata": sorted({row["category"] for row in train_rows}),
        "inputs": {
            "family": bind(args.family),
            "execution": bind(args.execution),
            "corpus": bind(args.train_reserve),
            "holdout_reserve_sealed": bind(args.holdout_reserve),
            "engine": bind(args.engine),
            "weights": bind(args.weights),
            "trainer": bind(args.trainer),
            "ranking_auditor": bind(args.ranking_auditor),
        },
        "shallow_contract": {
            "max_depth": 3,
            "root_candidates": 600,
            "threads": 1,
            "spec_top_n": 0,
            "timeout_seconds_per_search": 600,
            "nnue_output": fixed["output"],
        },
        "teacher_contract": {
            "max_depth": 7,
            "threads": 1,
            "spec_top_n": 0,
            "timeout_seconds_per_search": 600,
            "nnue_output": fixed["output"],
            "binary_sha256": sha256(args.engine),
            "weights_sha256": sha256(args.weights),
        },
        "candidate_contract": {
            "mode": "preregistered_candidate_union",
            "members": "depth-3 complete-root top 8 union every repeated depth-7 free bestmove",
            "free_repeats": 2,
            "fixed_root_repeats": 2,
            "maximum_moves_per_parent": 10,
            "top_k_after_depth7_reranking": 8,
            "normal_score_abs_max_cp": 10_000,
            "pair_selection": "top-vs-rest",
            "require_aa_signature_match": True,
            "require_exact_completed_search": True,
        },
        "training_contract": {
            "objective": "listwise-softmax",
            "temperature_cp": 400,
            "epochs": fixed["epochs"],
            "learning_rate": fixed["learning_rate"],
            "fresh_optimizer": True,
            "seed": fixed["seed"],
            "ranking_batch_pairs": 1,
            "ranking_parent_balanced": True,
            "nnue_output": fixed["output"],
            "architecture": {"input": 2420, "l1": 256, "l2": 32},
            "features": "v0.3.42 default; king_relative_b_small disabled",
        },
        "tools": {
            "runtime_preparer": bind(Path(__file__).resolve()),
            "shallow_runner": bind(args.shallow_runner),
            "label_preparer": bind(Path(__file__).resolve()),
            "label_runner": bind(args.label_runner),
            "pair_builder": bind(args.pair_builder),
            "train_runner": bind(args.train_runner),
        },
        "holdout_boundary": family["holdout_boundary"],
        "decision_contract": family["decision_contract"],
    }
    return document


def label(args: argparse.Namespace) -> dict[str, Any]:
    runtime_document = read(args.runtime_preregistration)
    shallow = read(args.shallow_teacher)
    require(
        runtime_document.get("schema") == RUNTIME_SCHEMA
        and runtime_document.get("status") == "frozen_before_shallow_labels",
        "unexpected Q28 runtime preregistration",
    )
    require(
        shallow.get("schema") == "sekirei.root-rank-teacher-corpus.v1"
        and len(shallow.get("rows", [])) == runtime_document["parents"],
        "incomplete Q28 shallow labels",
    )
    require(
        shallow.get("source_corpus", {}).get("sha256") == runtime_document["inputs"]["corpus"]["sha256"],
        "shallow corpus differs from runtime contract",
    )
    require(
        shallow.get("teacher", {}).get("binary_sha256") == runtime_document["inputs"]["engine"]["sha256"]
        and shallow.get("teacher", {}).get("weights_sha256") == runtime_document["inputs"]["weights"]["sha256"],
        "shallow runtime differs from frozen contract",
    )
    document = {
        **runtime_document,
        "schema": LABEL_SCHEMA,
        "status": "frozen_before_train_labels",
        "inputs": {**runtime_document["inputs"], "shallow_teacher": bind(args.shallow_teacher)},
        "runtime_preregistration": bind(args.runtime_preregistration),
    }
    return document


def write_immutable(path: Path, document: dict[str, Any]) -> None:
    encoded = json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    if path.exists() and path.read_text(encoding="utf-8") != encoded:
        raise ValueError("existing preregistration differs")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(encoded, encoding="utf-8")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    runtime_parser = subparsers.add_parser("runtime")
    runtime_parser.add_argument("--family", type=Path, required=True)
    runtime_parser.add_argument("--execution", type=Path, required=True)
    runtime_parser.add_argument("--train-reserve", type=Path, required=True)
    runtime_parser.add_argument("--holdout-reserve", type=Path, required=True)
    runtime_parser.add_argument("--engine", type=Path, required=True)
    runtime_parser.add_argument("--weights", type=Path, required=True)
    runtime_parser.add_argument("--trainer", type=Path, required=True)
    runtime_parser.add_argument("--ranking-auditor", type=Path, required=True)
    runtime_parser.add_argument("--shallow-runner", type=Path, required=True)
    runtime_parser.add_argument("--label-runner", type=Path, required=True)
    runtime_parser.add_argument("--pair-builder", type=Path, required=True)
    runtime_parser.add_argument("--train-runner", type=Path, required=True)
    runtime_parser.add_argument("--output", type=Path, required=True)
    label_parser = subparsers.add_parser("labels")
    label_parser.add_argument("--runtime-preregistration", type=Path, required=True)
    label_parser.add_argument("--shallow-teacher", type=Path, required=True)
    label_parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        document = runtime(args) if args.command == "runtime" else label(args)
        write_immutable(args.output, document)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    print(json.dumps({"schema": document["schema"], "status": document["status"]}, sort_keys=True))


if __name__ == "__main__":
    main()
