#!/usr/bin/env python3
"""Freeze Q21p's depth-7 ranking-label transfer pilot before measurement."""

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


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def read(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def prepare(args: argparse.Namespace) -> dict[str, Any]:
    q21m = read(args.q21m_preregistration)
    q21o = read(args.q21o_decision)
    superseded = read(args.supersedes_decision)
    corpus = read(args.corpus)
    shallow = read(args.shallow_teacher)
    require(
        q21m.get("schema") == "sekirei.q21m-pairwise-pilot-preregistration.v1",
        "Q21p requires Q21m's frozen pilot",
    )
    require(
        q21o.get("schema") == "sekirei.q21o-teacher-contract-decision.v1"
        and q21o.get("status") == "pass",
        "Q21p requires Q21o PASS",
    )
    require(
        superseded.get("schema") == "sekirei.q21p-depth7-ranking-pilot-decision.v1"
        and superseded.get("status") == "fail"
        and superseded.get("q21p_experiment_complete") is True,
        "Q21p v2 requires the preserved v1 FAIL decision",
    )
    selected = q21o["selected_contract"]
    require(
        selected.get("arm") == "depth7"
        and selected.get("max_depth") == 7
        and selected.get("threads") == 1
        and selected.get("spec_top_n") == 0
        and selected.get("nnue_output") == "residual-material",
        "Q21o selected contract is not the expected depth-7 contract",
    )
    require(
        corpus.get("schema") == "sekirei.q21m-ranking-train-corpus.v1"
        and corpus.get("selection", {}).get("positions") == 18,
        "Q21m train corpus is not the frozen 18-parent corpus",
    )
    require(
        shallow.get("schema") == "sekirei.root-rank-teacher-corpus.v1"
        and len(shallow.get("rows", [])) == 18
        and all(row.get("complete_legal_root_set") is True for row in shallow["rows"]),
        "Q21m shallow teacher roots are incomplete",
    )
    require(q21m["train_corpus"]["sha256"] == sha256(args.corpus), "Q21m corpus SHA mismatch")
    require(selected["weights_sha256"] == sha256(args.weights), "Q21o teacher weights SHA mismatch")
    require(selected["binary_sha256"] == sha256(args.engine), "Q21o engine binary SHA mismatch")

    training = q21m["training_contract"]
    validation = q21m["validation_contract"]
    document = {
        "schema": "sekirei.q21p-depth7-ranking-pilot-preregistration.v2",
        "status": "frozen_before_depth7_labels",
        "diagnostic_only": True,
        "strength_claim": False,
        "hypothesis": (
            "Replacing Q21m's shallow depth-3 ranking labels with the Q21o-selected depth-7 "
            "teacher contract transfers root ordering to the same student and frozen validation."
        ),
        "single_factor": "teacher-label generation contract: depth3 complete root -> depth7 preregistered candidate union",
        "correctness_fix": {
            "scope": "ranking loss now evaluates material plus NNUE residual in residual-material mode",
            "hyperparameter_change": False,
            "reason": (
                "v1 optimized NNUE residual alone while validation and inference ranked material plus residual; "
                "v2 restores the already-declared evaluator contract"
            ),
            "superseded_decision": bind(args.supersedes_decision),
        },
        "parents": 18,
        "candidate_contract": {
            "mode": "preregistered_candidate_union",
            "members": "Q21m depth-3 top 8 union every depth-7 free bestmove repeat",
            "maximum_moves_per_parent": 10,
            "free_repeats": 2,
            "fixed_root_repeats": 2,
            "top_k_after_depth7_reranking": 8,
            "pair_selection": "adjacent",
            "normal_score_abs_max_cp": 10_000,
            "require_exact_completed_search": True,
            "require_aa_signature_match": True,
        },
        "teacher_contract": {
            **selected,
            "cold_process_per_search": True,
            "timeout_seconds_per_search": 600,
        },
        "training_contract": {
            "initial_weights_sha256": selected["weights_sha256"],
            "fresh_optimizer": True,
            "init_seed": 42,
            "learning_rate": 0.0001,
            "epochs": training["epochs"],
            "ranking_parent_balanced": training["ranking_parent_balanced"],
            "ranking_batch_pairs": training["ranking_batch_pairs"],
            "nnue_output": training["nnue_output"],
            "no_scalar_targets": training["no_scalar_targets"],
        },
        "validation_contract": {
            **validation,
            "parent_chosen_move_change_is_reported_not_gated": True,
        },
        "authorization": {
            "screen_pass": "Q21p complete; authorize only the preregistered 32-game development match",
            "screen_fail": "reject candidate; no development match and no Q20",
            "q20_authorized": False,
        },
        "inputs": {
            "q21m_preregistration": bind(args.q21m_preregistration),
            "q21o_decision": bind(args.q21o_decision),
            "supersedes_decision": bind(args.supersedes_decision),
            "corpus": bind(args.corpus),
            "shallow_teacher": bind(args.shallow_teacher),
            "weights": bind(args.weights),
            "engine": bind(args.engine),
            "trainer": bind(args.trainer),
            "ranking_auditor": bind(args.auditor),
        },
        "tools": {
            "preparer": bind(Path(__file__).resolve()),
            "runner": bind(args.runner),
            "pair_builder": bind(args.pair_builder),
            "finalizer": bind(args.finalizer),
        },
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return document


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in (
        "q21m-preregistration",
        "q21o-decision",
        "supersedes-decision",
        "corpus",
        "shallow-teacher",
        "weights",
        "engine",
        "trainer",
        "auditor",
        "runner",
        "pair-builder",
        "finalizer",
        "output",
    ):
        parser.add_argument(f"--{name}", type=Path, required=True)
    args = parser.parse_args()
    try:
        document = prepare(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    print(json.dumps({"status": document["status"], "parents": document["parents"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
