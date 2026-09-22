#!/usr/bin/env python3
"""Freeze Q25h's mixed cp/mate terminal-order label protocol."""

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
    result = dict(prior)
    result.update(
        {
            "status": "frozen_before_score_blind_parent_selection",
            "family_id": "q25h-yaneuraou-v900-suisho5-multipv128-terminal-ordinal-v1",
            "hypothesis": (
                "retain Q25e's external teacher, score-blind parent eligibility, and uniform pairwise recipe, "
                "but represent a decisive mate versus ordinary cp score as an explicit ordinal relation instead "
                "of discarding the parent or inventing a centipawn gap"
            ),
            "single_factor": "mixed cp/mate label semantics: explicit terminal ordinal pairs",
            "pair_semantics": {
                "allow_mixed_terminal_ordinal": True,
                "ordinary_cp": "strict centipawn pair with its true cp gap",
                "mate_only": "strict mate ordinal pair with teacher_order_margin=1",
                "mixed_cp_mate": "positive mate > ordinary cp > negative mate; terminal_ordinal with teacher_order_margin=1",
                "forbidden": ["invented mate centipawn gap", "tied ordinal pair"],
            },
            "inputs": {
                "generator": bind(Path(__file__).resolve()),
                "q25e_family": bind(q25e_family),
            },
            "q25h_contract": {
                "baseline_protocol": "Q25e excludes mixed cp/mate roots without two cp moves",
                "candidate_protocol": "Q25e plus terminal ordinal pairs for mixed cp/mate roots",
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
