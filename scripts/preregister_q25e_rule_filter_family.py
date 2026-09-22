#!/usr/bin/env python3
"""Freeze Q25e's checked-position exclusion as one score-blind change."""

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


def build(q25d_family: Path) -> dict[str, Any]:
    prior = json.loads(q25d_family.read_text(encoding="utf-8"))
    require(
        prior.get("schema") == SCHEMA
        and prior.get("status") == "frozen_before_score_blind_parent_selection",
        "Q25d family contract is not frozen",
    )
    rule = dict(prior["rule_only_eligibility"])
    require(rule.get("exclude_in_check") is not True, "Q25e change already present")
    rule["exclude_in_check"] = True
    rule["purpose"] = (
        "reserve only non-check positions that can form a strict root pair before any teacher score is read"
    )
    result = dict(prior)
    result.update(
        {
            "status": "frozen_before_score_blind_parent_selection",
            "family_id": "q25e-yaneuraou-v900-suisho5-multipv128-mate-ordinal-rule-filter-v1",
            "hypothesis": (
                "retain Q25d's external label protocol but exclude checked parents using only rule facts, "
                "because Q25d's sole incomplete shallow root was a checked two-legal-move position"
            ),
            "single_factor": "rule-only parent eligibility: exclude positions where side to move is in check",
            "rule_only_eligibility": rule,
            "inputs": {
                "generator": bind(Path(__file__).resolve()),
                "q25d_family": bind(q25d_family),
            },
            "q25e_contract": {
                "baseline_protocol": "Q25d rule-only minimum legal moves=2 and exclude mate in one",
                "candidate_protocol": "Q25d plus exclude_in_check=true before any self or teacher score",
                "all_other_teacher_label_and_training_dimensions": "unchanged from Q25d",
            },
        }
    )
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--q25d-family", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = build(args.q25d_family)
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
