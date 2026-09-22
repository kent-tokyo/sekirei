#!/usr/bin/env python3
"""Freeze Q25g's teacher-gap confidence weighting as one new family factor."""

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


def build(q25e_family: Path) -> dict[str, Any]:
    prior = json.loads(q25e_family.read_text(encoding="utf-8"))
    require(
        prior.get("schema") == SCHEMA
        and prior.get("status") == "frozen_before_score_blind_parent_selection",
        "Q25e family contract is not frozen",
    )
    training = dict(prior["fixed_training"])
    require(training.get("objective") == "pairwise", "Q25g requires Q25e pairwise training")
    require("pair_weighting" not in training, "Q25g factor is already present")
    training["pair_weighting"] = "capped-teacher-gap"
    result = dict(prior)
    result.update(
        {
            "status": "frozen_before_score_blind_parent_selection",
            "family_id": "q25g-yaneuraou-v900-suisho5-multipv128-gap-weight-v1",
            "hypothesis": (
                "retain Q25e's score-blind parent eligibility and external label protocol, "
                "but weight ordinary cp pair updates by a bounded teacher gap because the historical "
                "pairwise path recorded every gap while treating one-centipawn and large gaps equally"
            ),
            "single_factor": "pairwise update weighting: capped external teacher centipawn gap",
            "fixed_training": training,
            "gap_weighting_contract": {
                "mode": "capped-teacher-gap",
                "ordinary_cp_weight": "clamp(teacher_score_gap_cp / 400, 0.05, 1.0)",
                "mate_ordinal_weight": 1.0,
                "selection_used_scores": False,
                "rationale": "400cp is the already fixed ranking-temperature scale; this does not create a centipawn value for mate labels",
            },
            "inputs": {
                "generator": bind(Path(__file__).resolve()),
                "q25e_family": bind(q25e_family),
            },
            "q25g_contract": {
                "baseline_protocol": "Q25e uniform pairwise updates",
                "candidate_protocol": "Q25e plus capped teacher-gap pair weights during pairwise updates",
                "all_other_teacher_label_and_training_dimensions": "unchanged from Q25e",
                "holdout_visibility": "sealed until candidate artifact and thresholds are frozen",
            },
        }
    )
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--q25e-family", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = build(args.q25e_family)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.parent.mkdir(parents=True, exist_ok=False)
    args.output.write_text(
        json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps({"status": result["status"], "family_id": result["family_id"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
