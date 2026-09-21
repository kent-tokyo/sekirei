#!/usr/bin/env python3
"""Fail-closed finalizer for the Q21p v3 fresh-holdout retry."""

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
        raise ValueError(f"missing artifact: {path}")
    return {"path": str(path), "sha256": sha256(path)}


def read(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def finalize(args: argparse.Namespace) -> dict[str, Any]:
    prereg = read(args.preregistration)
    measurements = read(args.measurements)
    label_audit = read(args.label_audit)
    pairs = read(args.pairs)
    pair_audit = read(args.pair_audit)
    training = read(args.training_metadata)
    mode = read(args.output_mode_metadata)
    baseline = read(args.baseline_audit)
    candidate = read(args.candidate_audit)
    screen = read(args.screen)
    require(
        prereg.get("schema") == "sekirei.q21p-depth7-ranking-pilot-preregistration.v4"
        and prereg.get("status") == "frozen_before_depth7_validation_labels_and_candidate_training",
        "unexpected Q21p v3 preregistration",
    )
    require(
        measurements.get("schema") == "sekirei.q21p-depth7-label-measurements.v1"
        and measurements.get("preregistration", {}).get("sha256") == sha256(args.preregistration)
        and len(measurements.get("rows", [])) == prereg["parents"],
        "Q21p v3 measurements are incomplete or unbound",
    )
    require(
        label_audit.get("schema") == "sekirei.q21p-depth7-label-audit.v1"
        and label_audit.get("status") == "pass"
        and label_audit.get("parents") == prereg["parents"]
        and label_audit.get("parents_with_pairs", 0) > 0
        and label_audit.get("all_aa_deterministic") is True,
        "Q21p v3 label audit did not pass",
    )
    measurement_sha = sha256(args.measurements)
    require(
        pairs.get("schema") == "sekirei.root-rank-pairs.v1"
        and pairs.get("source_contract", {}).get("candidate_source_sha256") == measurement_sha
        and len(pairs.get("pairs", [])) == label_audit["pairs"],
        "validation pairs are not bound to the audited labels",
    )
    require(
        pair_audit.get("schema") == "sekirei.root-rank-pair-audit.v1"
        and pair_audit.get("pairs_verified") == label_audit["pairs"]
        and pair_audit.get("parent_positions") == label_audit["parents_with_pairs"],
        "Rust pair audit did not verify the validation corpus",
    )
    contract = prereg["training_contract"]
    require(
        training.get("schema") == "sekirei.ranking-training-run.v1"
        and training.get("pairs_path") == prereg["inputs"]["train_pairs"]["path"]
        and training.get("epochs") == contract["epochs"]
        and training.get("ranking_parent_balanced") is True
        and abs(float(training.get("learning_rate")) - contract["learning_rate"]) < 1e-9,
        "candidate training differs from the frozen recipe",
    )
    require(
        mode.get("format") == "sekirei-nnue-output-v1"
        and mode.get("nnue_output") == "residual-material",
        "candidate output mode mismatch",
    )
    for name, audit in (("baseline", baseline), ("candidate", candidate)):
        require(
            audit.get("schema") == "sekirei.root-rank-pair-audit.v1"
            and audit.get("pairs_verified") == label_audit["pairs"]
            and audit.get("model_diagnostic", {}).get("pairs_scored") == label_audit["pairs"],
            f"{name} validation audit is incomplete",
        )
    require(
        baseline["model_diagnostic"]["checkpoint"] == prereg["inputs"]["weights"]["path"],
        "baseline audit did not use the frozen initial checkpoint",
    )
    require(
        screen.get("schema") == "sekirei.q21i-imitation-screen.v1"
        and screen.get("inputs", {}).get("baseline", {}).get("sha256") == sha256(args.baseline_audit)
        and screen.get("inputs", {}).get("candidate", {}).get("sha256") == sha256(args.candidate_audit),
        "screen inputs differ from the audited fresh hold-out",
    )
    require(
        screen.get("parents") == label_audit["parents_with_pairs"],
        "screen parent count differs from the normal-cp parent set",
    )
    passed = screen.get("status") == "screen_pass"
    require(screen.get("status") in {"screen_pass", "screen_fail"}, "unsupported screen status")
    return {
        "schema": "sekirei.q21p-depth7-ranking-pilot-decision.v4",
        "status": "pass" if passed else "fail",
        "q21p_experiment_complete": True,
        "q21p_success": passed,
        "diagnostic_only": True,
        "strength_claim": False,
        "training": {
            "parents": training["parent_groups"],
            "pairs": training["pairs"],
            "epochs": training["epochs"],
            "learning_rate": training["learning_rate"],
            "final_pairwise_loss": training["mean_pairwise_loss_final"],
        },
        "validation": {
            "parents": screen["parents"],
            "pairs": label_audit["pairs"],
            "baseline_mean_parent_rank_loss_cp": screen["baseline"]["mean_parent_rank_loss_cp"],
            "candidate_mean_parent_rank_loss_cp": screen["candidate"]["mean_parent_rank_loss_cp"],
            "rank_loss_reduction": screen["rank_loss_reduction"],
            "baseline_major_blunders": screen["baseline"]["major_blunders_ge_300cp"],
            "candidate_major_blunders": screen["candidate"]["major_blunders_ge_300cp"],
        },
        "candidate_rejected": not passed,
        "development_match_authorized": passed,
        "q20_authorized": False,
        "next_action": (
            "run only the preregistered 32-game development match"
            if passed else "reject this candidate and do not run a development match or Q20"
        ),
        "tool_provenance": {
            "preregistered_finalizer_sha256": prereg["tools"]["finalizer"]["sha256"],
            "executed_finalizer_sha256": sha256(Path(__file__).resolve()),
            "contract_preserving_fix_after_measurement": (
                prereg["tools"]["finalizer"]["sha256"] != sha256(Path(__file__).resolve())
            ),
            "fix_scope": (
                "accept the audited ordinary-cp parent count after explicit mate-like parent "
                "separation; screen thresholds and candidate metrics are unchanged"
            ),
        },
        "artifacts": {
            name: bind(path)
            for name, path in {
                "preregistration": args.preregistration,
                "measurements": args.measurements,
                "label_audit": args.label_audit,
                "pairs": args.pairs,
                "pair_audit": args.pair_audit,
                "candidate": args.candidate,
                "training_metadata": args.training_metadata,
                "output_mode_metadata": args.output_mode_metadata,
                "baseline_audit": args.baseline_audit,
                "candidate_audit": args.candidate_audit,
                "screen": args.screen,
                "finalizer": Path(__file__).resolve(),
            }.items()
        },
    }


def report(document: dict[str, Any]) -> str:
    validation = document["validation"]
    return "\n".join((
        "# Q21p depth-7 ranking transfer retry",
        "",
        "Fresh independent-validation screen; not a strength result.",
        "",
        f"- Status: `{document['status']}`.",
        f"- Training: {document['training']['parents']} parents, {document['training']['pairs']} pairs, "
        f"{document['training']['epochs']} epochs at lr={document['training']['learning_rate']}.",
        f"- Mean parent rank loss: {validation['baseline_mean_parent_rank_loss_cp']:.3f} -> "
        f"{validation['candidate_mean_parent_rank_loss_cp']:.3f} "
        f"({validation['rank_loss_reduction'] * 100:.2f}% reduction).",
        f"- Major >=300cp blunders: {validation['baseline_major_blunders']} -> "
        f"{validation['candidate_major_blunders']}.",
        f"- Development match authorized: {document['development_match_authorized']}.",
        "- Q20 remains unauthorized.",
        "",
    ))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in (
        "preregistration", "measurements", "label-audit", "pairs", "pair-audit",
        "candidate", "training-metadata", "output-mode-metadata", "baseline-audit",
        "candidate-audit", "screen", "output", "report",
    ):
        parser.add_argument(f"--{name}", type=Path, required=True)
    args = parser.parse_args()
    try:
        document = finalize(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    args.report.write_text(report(document), encoding="utf-8")
    print(json.dumps({
        "status": document["status"],
        "rank_loss_reduction": document["validation"]["rank_loss_reduction"],
        "development_match_authorized": document["development_match_authorized"],
    }, sort_keys=True))
    return 0 if document["status"] == "pass" else 1


if __name__ == "__main__":
    raise SystemExit(main())
