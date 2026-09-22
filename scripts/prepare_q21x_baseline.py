#!/usr/bin/env python3
"""Freeze Q21x's runtime and train-only baseline before any new label.

Q21w froze membership before labels, but its target binaries may later be
rebuilt in place.  Q21x therefore creates a new immutable runtime binding
without modifying Q21w or opening the reserved hold-out scores.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any


SCHEMA = "sekirei.q21x-train-preregistration.v1"
Q21W_SCHEMA = "sekirei.q21w-independent-coverage-preregistration.v1"


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


def positions(document: dict[str, Any], expected_schema: str, count: int) -> list[dict[str, Any]]:
    require(document.get("schema") == expected_schema, f"unexpected {expected_schema} input")
    rows = document.get("positions")
    require(isinstance(rows, list) and len(rows) == count, f"expected {count} positions")
    return rows


def source_keys(rows: list[dict[str, Any]]) -> set[str]:
    values = {row.get("source", {}).get("source_key") for row in rows}
    require(None not in values and len(values) == len(rows), "CSA sources are not unique")
    return values  # type: ignore[return-value]


def identities(rows: list[dict[str, Any]]) -> set[str]:
    values = {row.get("source", {}).get("derived_group") for row in rows}
    require(None not in values and len(values) == len(rows), "derived groups are not unique")
    return values  # type: ignore[return-value]


def prepare(args: argparse.Namespace) -> dict[str, Any]:
    q21w = read(args.q21w_preregistration)
    train = read(args.train_reserve)
    holdout = read(args.holdout_reserve)
    require(
        q21w.get("schema") == Q21W_SCHEMA
        and q21w.get("status") == "frozen_before_any_teacher_label",
        "unexpected Q21w preregistration",
    )
    for name, path in (
        ("train_reserve", args.train_reserve),
        ("holdout_reserve", args.holdout_reserve),
    ):
        require(q21w["artifacts"][name]["sha256"] == sha256(path), f"{name} SHA mismatch")
    train_rows = positions(train, "sekirei.q21w-train-reserve.v1", 72)
    holdout_rows = positions(holdout, "sekirei.q21w-holdout-reserve.v1", 18)
    require(not (source_keys(train_rows) & source_keys(holdout_rows)), "train/holdout source overlap")
    require(not (identities(train_rows) & identities(holdout_rows)), "train/holdout group overlap")
    contract = q21w["contract"]
    teacher = contract["teacher"]
    require(contract["train_parents"] == 72 and contract["holdout_parents"] == 18, "Q21w count drift")
    require(
        contract["objective"] == "listwise-softmax"
        and contract["temperature_cp"] == 400
        and contract["epochs"] == 60,
        "Q21w training contract drift",
    )
    runtime = {
        "engine": bind(args.engine),
        "weights": bind(args.weights),
        "trainer": bind(args.trainer),
        "ranking_auditor": bind(args.ranking_auditor),
    }
    require(runtime["weights"]["sha256"] == teacher["weights_sha256"], "teacher weights drift")
    document = {
        "schema": SCHEMA,
        "status": "frozen_before_train_labels",
        "diagnostic_only": True,
        "strength_claim": False,
        "single_factor": "independent parent coverage only; architecture and features remain v0.3.42 defaults",
        "parents": 72,
        "strata": contract["strata"],
        "inputs": {
            "q21w_preregistration": bind(args.q21w_preregistration),
            "corpus": bind(args.train_reserve),
            "holdout_reserve_sealed": bind(args.holdout_reserve),
            **runtime,
        },
        "runtime_rebind": {
            "reason": "Q21w target binaries were rebuilt in place before labels; Q21x binds current artifacts without changing membership or looking at holdout scores",
            "q21w_teacher_binary_sha256": teacher["binary_sha256"],
            "q21x_teacher_binary_sha256": runtime["engine"]["sha256"],
            "membership_changed": False,
            "labels_existed_before_rebind": False,
        },
        "candidate_contract": {
            "mode": "preregistered_candidate_union",
            "members": "depth-3 complete-root top 8 union every repeated depth-7 free bestmove",
            "free_repeats": 2,
            "fixed_root_repeats": 2,
            "maximum_moves_per_parent": contract["candidate_limit"],
            "top_k_after_depth7_reranking": 8,
            "normal_score_abs_max_cp": 10_000,
            "pair_selection": "top-vs-rest",
            "require_aa_signature_match": True,
            "require_exact_completed_search": True,
        },
        "teacher_contract": {
            **teacher,
            "binary_sha256": runtime["engine"]["sha256"],
            "timeout_seconds_per_search": teacher["timeout_seconds"],
        },
        "training_contract": {
            "objective": contract["objective"],
            "temperature_cp": contract["temperature_cp"],
            "epochs": contract["epochs"],
            "learning_rate": contract["learning_rate"],
            "fresh_optimizer": True,
            "seed": contract["seed_for_initialization"],
            "ranking_batch_pairs": 1,
            "ranking_parent_balanced": True,
            "nnue_output": contract["nnue_output"],
            "architecture": {"input": 2420, "l1": 256, "l2": 32},
            "features": "v0.3.42 default; king_relative_b_small disabled",
        },
        "holdout_boundary": {
            "membership_sha256": sha256(args.holdout_reserve),
            "scores_inspected": False,
            "labels_generated": False,
            "candidate_must_be_frozen_before_holdout_labeling": True,
            "retired_selection_evidence": q21w["forbidden_selection_evidence"],
        },
        "screen_contract": q21w["stop_conditions"],
        "tools": {
            "preparer": bind(Path(__file__).resolve()),
            "label_runner": bind(args.label_runner),
            "pair_builder": bind(args.pair_builder),
            "train_runner": bind(args.train_runner),
            "holdout_preparer": bind(args.holdout_preparer),
            "screen_runner": bind(args.screen_runner),
        },
    }
    args.output_dir.mkdir(parents=True, exist_ok=True)
    output = args.output_dir / "train-preregistration.json"
    encoded = json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    if output.exists() and output.read_text(encoding="utf-8") != encoded:
        raise ValueError("existing Q21x preregistration differs")
    output.write_text(encoded, encoding="utf-8")
    return document


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--q21w-preregistration", type=Path, required=True)
    parser.add_argument("--train-reserve", type=Path, required=True)
    parser.add_argument("--holdout-reserve", type=Path, required=True)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--weights", type=Path, required=True)
    parser.add_argument("--trainer", type=Path, required=True)
    parser.add_argument("--ranking-auditor", type=Path, required=True)
    parser.add_argument("--label-runner", type=Path, required=True)
    parser.add_argument("--pair-builder", type=Path, required=True)
    parser.add_argument("--train-runner", type=Path, required=True)
    parser.add_argument("--holdout-preparer", type=Path, required=True)
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
