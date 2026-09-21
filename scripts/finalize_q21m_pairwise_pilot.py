#!/usr/bin/env python3
"""Fail-closed finalizer for the preregistered Q21m pairwise pilot."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def bind(path: Path) -> dict[str, str]:
    require(path.is_file(), f"missing artifact: {path}")
    return {"path": str(path), "sha256": sha256(path)}


def decision_for(screen_status: str) -> dict[str, Any]:
    require(screen_status in {"screen_pass", "screen_fail"}, "unsupported screen status")
    passed = screen_status == "screen_pass"
    return {
        "q21m_completed": not passed,
        "candidate_rejected": not passed,
        "development_match_authorized": passed,
        "q20_authorized": False,
    }


def finalize(args: argparse.Namespace) -> dict[str, Any]:
    prereg = read(args.preregistration)
    corpus = read(args.corpus)
    teacher_root = read(args.teacher_root)
    pairs = read(args.pairs)
    pairs_audit = read(args.pairs_audit)
    training = read(args.training_metadata)
    output_mode = read(args.output_mode_metadata)
    validation = read(args.validation_audit)
    screen = read(args.screen)

    require(
        prereg.get("schema") == "sekirei.q21m-pairwise-pilot-preregistration.v1"
        and prereg.get("status") == "frozen_before_teacher_labels",
        "unexpected Q21m preregistration",
    )
    require(
        prereg["train_corpus"]["sha256"] == sha256(args.corpus),
        "training corpus differs from preregistration",
    )
    require(
        corpus.get("schema") == "sekirei.q21m-ranking-train-corpus.v1"
        and corpus.get("selection", {}).get("positions") == 18,
        "invalid Q21m train corpus",
    )
    contract = prereg["teacher_label_contract"]
    require(
        teacher_root.get("schema") == "sekirei.root-rank-teacher-corpus.v1"
        and teacher_root.get("contract", {}).get("depth") == contract["depth"]
        and teacher_root.get("contract", {}).get("threads") == contract["threads"]
        and teacher_root.get("contract", {}).get("spec_top_n") == contract["spec_top_n"]
        and teacher_root.get("contract", {}).get("root_candidate_limit")
        == contract["root_candidate_limit"]
        and teacher_root.get("contract", {}).get("complete_legal_root_set") is True,
        "teacher-root contract mismatch",
    )
    require(
        teacher_root.get("source_corpus", {}).get("sha256") == sha256(args.corpus),
        "teacher root source differs from frozen corpus",
    )
    require(
        teacher_root.get("teacher", {}).get("weights_sha256")
        == prereg["fixed"]["teacher_weights"]["sha256"]
        and teacher_root.get("teacher", {}).get("nnue_output") == prereg["fixed"]["teacher_output"],
        "teacher identity differs from preregistration",
    )
    require(
        len(teacher_root.get("rows", [])) == 18
        and all(
            row.get("candidate_prefix_complete") is True
            and row.get("complete_legal_root_set") is True
            for row in teacher_root["rows"]
        ),
        "teacher root rows are incomplete",
    )
    require(
        pairs.get("schema") == "sekirei.root-rank-pairs.v1"
        and pairs.get("pair_selection") == contract["pair_selection"]
        and len(pairs.get("pairs", [])) == 90,
        "unexpected Q21m pair corpus",
    )
    require(
        pairs_audit.get("schema") == "sekirei.root-rank-pair-audit.v1"
        and pairs_audit.get("pairs_verified") == 90
        and pairs_audit.get("parent_positions") == 18
        and pairs_audit.get("training_performed") is False,
        "pair audit did not verify the frozen training set",
    )
    training_contract = prereg["training_contract"]
    require(
        training.get("schema") == "sekirei.ranking-training-run.v1"
        and training.get("pairs_path") == str(args.pairs)
        and training.get("pairs") == 90
        and training.get("parent_groups") == 18
        and training.get("epochs") == training_contract["epochs"]
        and training.get("ranking_parent_balanced") is True
        and training.get("ranking_batch_pairs") == training_contract["ranking_batch_pairs"]
        and abs(float(training.get("learning_rate")) - prereg["fixed"]["learning_rate"]) < 1e-9,
        "training metadata differs from preregistration",
    )
    require(
        output_mode.get("format") == "sekirei-nnue-output-v1"
        and output_mode.get("nnue_output") == "residual-material"
        and output_mode.get("baseline") == "material-v1",
        "candidate output-mode metadata mismatch",
    )
    require(
        validation.get("schema") == "sekirei.root-rank-pair-audit.v1"
        and validation.get("model_diagnostic", {}).get("pairs_scored") == 58_451,
        "validation audit is incomplete",
    )
    require(
        screen.get("schema") == "sekirei.q21i-imitation-screen.v1"
        and screen.get("inputs", {}).get("baseline", {}).get("sha256")
        == prereg["validation_contract"]["baseline_audit"]["sha256"]
        and screen.get("inputs", {}).get("candidate", {}).get("sha256")
        == sha256(args.validation_audit),
        "screen inputs differ from preregistration or audit",
    )
    decision = decision_for(screen["status"])
    if screen["status"] == "screen_fail":
        require(screen.get("checks", {}).get("rank_loss_reduction_pass") is False,
                "screen failure has no failed rank-loss criterion")
    return {
        "schema": "sekirei.q21m-pairwise-pilot-decision.v1",
        "status": "complete" if decision["q21m_completed"] else "screen_pass_waiting_development_match",
        "diagnostic_only": True,
        "strength_claim": False,
        "single_factor": prereg["single_factor"],
        "training": {
            "parents": training["parent_groups"],
            "pairs": training["pairs"],
            "epochs": training["epochs"],
            "final_pairwise_loss": training["mean_pairwise_loss_final"],
        },
        "validation": {
            "parents": screen["parents"],
            "baseline_mean_parent_rank_loss_cp": screen["baseline"]["mean_parent_rank_loss_cp"],
            "candidate_mean_parent_rank_loss_cp": screen["candidate"]["mean_parent_rank_loss_cp"],
            "rank_loss_reduction": screen["rank_loss_reduction"],
            "baseline_major_blunders": screen["baseline"]["major_blunders_ge_300cp"],
            "candidate_major_blunders": screen["candidate"]["major_blunders_ge_300cp"],
            "screen_status": screen["status"],
        },
        "decision": decision,
        "interpretation": (
            "Training loss decreased, but the frozen validation parent choices did not improve. "
            "Reject this pairwise pilot without a development match or Q20."
            if screen["status"] == "screen_fail"
            else "Ranking screen passed; only the preregistered development match is authorized."
        ),
        "artifacts": {
            name: bind(path)
            for name, path in {
                "preregistration": args.preregistration,
                "corpus": args.corpus,
                "teacher_root": args.teacher_root,
                "pairs": args.pairs,
                "pairs_audit": args.pairs_audit,
                "candidate": args.candidate,
                "training_metadata": args.training_metadata,
                "output_mode_metadata": args.output_mode_metadata,
                "validation_audit": args.validation_audit,
                "screen": args.screen,
                "finalizer": Path(__file__).resolve(),
            }.items()
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in (
        "preregistration", "corpus", "teacher-root", "pairs", "pairs-audit",
        "candidate", "training-metadata", "output-mode-metadata", "validation-audit",
        "screen", "output",
    ):
        parser.add_argument(f"--{name}", type=Path, required=True)
    args = parser.parse_args()
    try:
        document = finalize(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.write_text(
        json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps(document["decision"] | {"status": document["status"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
