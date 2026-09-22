#!/usr/bin/env python3
"""Freeze Q25c's wider, free-MultiPV external-label family.

Q25a proved that the pinned teacher is A/A-stable, but MultiPV=4 could not
cover every frozen self-candidate union.  Q25c changes only that coverage
protocol: the same local-only YaneuraOu/Suisho5 teacher is queried with
MultiPV=16.  Model shape, initial weights, optimizer, and screening rules are
carried forward unchanged.  Parent selection is deliberately deferred to the
existing score-blind Q25a-compatible reserve builder.
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


def build(q25a_family: Path) -> dict[str, Any]:
    prior = json.loads(q25a_family.read_text(encoding="utf-8"))
    require(
        prior.get("schema") == SCHEMA
        and prior.get("status") == "frozen_before_score_blind_parent_selection",
        "Q25a family contract is not frozen",
    )
    external = dict(prior["external_teacher"])
    options = dict(external["usi_options"])
    require(str(options.get("MultiPV")) == str(external["multipv"]), "Q25a MultiPV contract mismatch")
    require(int(external["multipv"]) < 16, "Q25c requires a wider MultiPV contract")
    external["multipv"] = 16
    options["MultiPV"] = "16"
    external["usi_options"] = options

    result = dict(prior)
    result.update(
        {
            "status": "frozen_before_score_blind_parent_selection",
            "family_id": "q25c-yaneuraou-v900-suisho5-multipv16-labels-v1",
            "hypothesis": (
                "replace only the depth-7 self-teacher ranking labels with the pinned "
                "YaneuraOu V9.00 plus Suisho5 free-MultiPV=16 labels"
            ),
            "single_factor": "external free-MultiPV label coverage (4 to 16)",
            "external_teacher": external,
            "inputs": {
                "generator": bind(Path(__file__).resolve()),
                "q25a_family": bind(q25a_family),
            },
            "q25c_contract": {
                "baseline_protocol": "Q25a A/A-stable depth-10 free MultiPV=4",
                "candidate_protocol": "A/A-stable depth-10 free MultiPV=16",
                "fixed_dimensions": [
                    "teacher binary and weights",
                    "teacher depth, threads, hash, and non-MultiPV USI options",
                    "model shape and features",
                    "initial weights and optimizer recipe",
                    "static and same-time screening thresholds",
                ],
                "required_train_coverage": "every frozen parent has at least two ordinary cp labels",
                "holdout_visibility": "sealed until the candidate artifact and thresholds are frozen",
            },
        }
    )
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--q25a-family", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = build(args.q25a_family)
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
