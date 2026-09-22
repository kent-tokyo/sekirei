#!/usr/bin/env python3
"""Preregister the one external-label candidate family warranted by Q25."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

from prepare_q25_external_teacher_calibration import bind


SCHEMA = "sekirei.q25a-external-label-family-preregistration.v1"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def build(
    q25_preregistration: Path,
    q25_decision: Path,
    q21x_train_preregistration: Path,
    q21x_holdout_decision: Path,
) -> dict[str, Any]:
    q25 = json.loads(q25_preregistration.read_text(encoding="utf-8"))
    decision = json.loads(q25_decision.read_text(encoding="utf-8"))
    q21x_train = json.loads(q21x_train_preregistration.read_text(encoding="utf-8"))
    q21x_holdout = json.loads(q21x_holdout_decision.read_text(encoding="utf-8"))
    require(
        decision.get("status") == "external_teacher_candidate_family_warranted"
        and decision.get("external_teacher_family_warranted") is True,
        "Q25 did not warrant an external-label family",
    )
    require(q25.get("status") == "frozen_before_any_q25_label", "Q25 contract is not frozen")
    require(q21x_holdout.get("status") == "fail", "Q21x baseline decision is not the rejected control")
    training = q21x_train["training_contract"]
    external = q25["external_teacher"]
    return {
        "schema": SCHEMA,
        "status": "frozen_before_score_blind_parent_selection",
        "diagnostic_only": True,
        "strength_claim": False,
        "candidate_adopted": False,
        "q20_authorized": False,
        "family_id": "q25a-yaneuraou-v900-suisho5-labels-v1",
        "hypothesis": (
            "replace only the depth-7 self-teacher ranking labels with the Q25-pinned "
            "YaneuraOu V9.00 plus Suisho5 labels"
        ),
        "single_factor": "teacher label source",
        "fixed_model": {
            "l1": 256,
            "l2": 32,
            "features": "default",
            "nnue_output": "residual-material",
            "initial_weights_sha256": q21x_train["inputs"]["weights"]["sha256"],
        },
        "fixed_training": {
            "objective": training["objective"],
            "temperature_cp": training["temperature_cp"],
            "optimizer": "fresh Adam",
            "epochs": training["epochs"],
            "learning_rate": training["learning_rate"],
            "seed": training["seed"],
            "ranking_parent_balanced": training["ranking_parent_balanced"],
        },
        "external_teacher": {
            "name": external["name"],
            "revision": external["revision"],
            "release_tag": external["release_tag"],
            "weights_release_tag": external["weights_release_tag"],
            "engine_sha256": q25["inputs"]["external_engine"]["sha256"],
            "weights_sha256": q25["inputs"]["external_weights"]["sha256"],
            "max_depth": external["max_depth"],
            "multipv": external["multipv"],
            "threads": external["threads"],
            "hash_mb": external["hash_mb"],
            "usi_options": external["usi_options"],
            "license_boundary": {
                "engine": external["engine_license"],
                "weights": external["weights_license"],
                "redistribute": False,
                "core_dependency": False,
                "use": "local label generation only",
            },
        },
        "new_data_boundary": {
            "selection": "score-blind before any self or external label",
            "strata": q25["strata"],
            "train_parents_per_stratum": 8,
            "holdout_parents_per_stratum": 2,
            "train_parents": 72,
            "holdout_parents": 18,
            "one_parent_per_csa_source": True,
            "exclude": [
                "all Q20 positions and CSA sources",
                "all Q21/Q21x/Q21y/Q21z positions and CSA sources",
                "all nine Q25 calibration parents and CSA sources",
                "exact and color-mirrored SFEN identities from every set above",
            ],
            "holdout_visibility": "open once only after the candidate artifact and thresholds are frozen",
        },
        "screen": {
            "static_primary": "depth-10 external-teacher direct top regret",
            "mean_direct_top_regret_reduction_minimum": 0.10,
            "top1_matches_must_not_decrease": True,
            "major_regret_ge_300_must_not_increase": True,
            "same_time_against_material": {
                "mean_regret_must_not_exceed_material": True,
                "top1_matches_must_not_be_below_material": True,
                "major_regret_must_not_exceed_material": True,
            },
            "all_static_and_same_time_conditions_required": True,
        },
        "after_screen": {
            "pass": "freeze at most one candidate and proceed to Q27 M 32-game screen",
            "fail": "reject the family; retain current evaluator and continue to Q26",
            "q20": "not authorized until Q27 passes",
            "competitor_match": "not authorized until Q20 passes",
        },
        "inputs": {
            "generator": bind(Path(__file__).resolve()),
            "q25_preregistration": bind(q25_preregistration),
            "q25_decision": bind(q25_decision),
            "q21x_train_preregistration": bind(q21x_train_preregistration),
            "q21x_holdout_decision": bind(q21x_holdout_decision),
        },
        "prohibitions": [
            "no architecture or feature change in this family",
            "no tuning on the Q25 calibration parents",
            "no reuse of an opened Q21 holdout",
            "no external binary or weights in the Rust core or release artifacts",
            "no candidate adoption from training loss or Q25 calibration alone",
        ],
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--q25-preregistration", type=Path, required=True)
    parser.add_argument("--q25-decision", type=Path, required=True)
    parser.add_argument("--q21x-train-preregistration", type=Path, required=True)
    parser.add_argument("--q21x-holdout-decision", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = build(
            args.q25_preregistration,
            args.q25_decision,
            args.q21x_train_preregistration,
            args.q21x_holdout_decision,
        )
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.write_text(
        json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps({"status": result["status"], "family_id": result["family_id"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
