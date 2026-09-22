#!/usr/bin/env python3
"""Freeze Q25d's score-blind mate-aware external-label protocol.

Q25c established that merely widening MultiPV is insufficient: terminal
mate-only positions have no ordinary centipawn label, while some wide tactical
unions need more than 16 principal variations. Q25d changes the *label
protocol* as one unit: free MultiPV=128 plus explicit cp-or-mate-ordinal
pairwise labels, with a score-blind minimum legal-move boundary.
"""

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


def build(q25c_family: Path) -> dict[str, Any]:
    prior = json.loads(q25c_family.read_text(encoding="utf-8"))
    require(
        prior.get("schema") == SCHEMA
        and prior.get("status") == "frozen_before_score_blind_parent_selection",
        "Q25c family contract is not frozen",
    )
    external = dict(prior["external_teacher"])
    options = dict(external["usi_options"])
    require(int(external["multipv"]) == 16, "Q25d must follow the rejected Q25c protocol")
    external["multipv"] = 128
    options["MultiPV"] = "128"
    external["usi_options"] = options
    training = dict(prior["fixed_training"])
    training["objective"] = "pairwise"
    training.pop("temperature_cp", None)

    result = dict(prior)
    result.update(
        {
            "status": "frozen_before_score_blind_parent_selection",
            "family_id": "q25d-yaneuraou-v900-suisho5-multipv128-mate-ordinal-v1",
            "hypothesis": (
                "replace depth-7 self labels with pinned external free-MultiPV=128 labels, "
                "retaining centipawn gaps only for cp pairs and strict ordinal pairs for mate-only roots"
            ),
            "single_factor": "external teacher label protocol (coverage plus explicit mate semantics)",
            "external_teacher": external,
            "fixed_training": training,
            "rule_only_eligibility": {
                "min_legal_moves": 2,
                "exclude_mate_in_one": True,
                "selection_used_scores": False,
                "purpose": "reserve only positions that can contain a strict root pair before any teacher score is read",
            },
            "q25d_contract": {
                "baseline_protocol": "Q25c free MultiPV=16 with ordinary-cp-only labels",
                "candidate_protocol": "A/A-stable free MultiPV=128 with cp-or-single-sign-mate ordinal pairs",
                "pair_semantics": {
                    "centipawn": "strict cp gap bounded by the existing source contract",
                    "mate_ordinal": "strict adjacent mate order uses teacher_order_margin=1 and no cp gap",
                    "forbidden": ["mixed cp/mate pair", "mixed mate signs", "tied mate-only candidate set"],
                },
                "required_train_coverage": "every frozen parent yields at least one strict audited pair",
                "holdout_visibility": "sealed until the candidate artifact and thresholds are frozen",
            },
            "inputs": {
                "generator": bind(Path(__file__).resolve()),
                "q25c_family": bind(q25c_family),
            },
        }
    )
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--q25c-family", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = build(args.q25c_family)
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
