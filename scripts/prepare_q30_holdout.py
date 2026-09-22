#!/usr/bin/env python3
"""Freeze both Q30 capacity candidates before opening the 18-parent hold-out."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

import prepare_q28_training as common


SCHEMA = "sekirei.q30-holdout-preregistration.v1"


def run(args: argparse.Namespace) -> dict:
    train = common.read(args.train_preregistration)
    decision = common.read(args.train_decision)
    reserve = common.read(args.holdout_reserve)
    common.require(
        train.get("schema") == "sekirei.q30-train-preregistration.v1"
        and train.get("status") == "frozen_before_train_labels",
        "unexpected Q30 train preregistration",
    )
    common.require(
        decision.get("schema") == "sekirei.q30-train-decision.v1"
        and decision.get("status") == "candidates_frozen"
        and decision.get("holdout_inspected") is False
        and decision.get("holdout_labels_generated") is False,
        "Q30 candidates were not frozen before holdout",
    )
    common.require(
        train["inputs"]["holdout_reserve_sealed"]["sha256"] == common.sha256(args.holdout_reserve),
        "Q30 holdout membership SHA mismatch",
    )
    rows = reserve.get("positions")
    common.require(reserve.get("schema") == "sekirei.q30-holdout-reserve.v1" and isinstance(rows, list) and len(rows) == 18, "invalid Q30 holdout")
    for field in ("source_key", "derived_group"):
        values = {row.get("source", {}).get(field) for row in rows}
        common.require(None not in values and len(values) == 18, f"Q30 holdout {field} is not unique")
    # The base depth-7 teacher is shared with the train labels.  The reduced
    # evaluator binary, however, is a *validation* input: it was not used to
    # make any label and therefore must be frozen here, before the sealed
    # hold-out is opened, rather than retroactively inferred from the train
    # manifest.
    shared_bindings = {
        "engine": args.teacher_engine,
        "weights": args.teacher_weights,
        "base_ranking_auditor": args.base_ranking_auditor,
        "reduced_ranking_auditor": args.reduced_ranking_auditor,
    }
    for name, path in shared_bindings.items():
        expected = train["inputs"].get(name)
        common.require(
            isinstance(expected, dict) and expected["sha256"] == common.sha256(path),
            f"Q30 {name} SHA mismatch",
        )
    common.require(
        train["inputs"]["engine"]["sha256"] == common.sha256(args.base_engine),
        "Q30 base engine must match the frozen teacher binary",
    )
    candidates = {"base": common.bind(args.base_candidate), "reduced": common.bind(args.reduced_candidate)}
    for arm, binding in candidates.items():
        common.require(decision["arms"][f"{arm}_256_32" if arm == "base" else "reduced_128_16"]["candidate"]["artifact"]["sha256"] == binding["sha256"], f"Q30 {arm} candidate SHA mismatch")
    return {
        "schema": SCHEMA, "status": "frozen_before_holdout_labels", "diagnostic_only": True,
        "strength_claim": False, "parents": 18, "single_factor": train["single_factor"],
        "strata": sorted({row["category"] for row in rows}),
        "inputs": {
            "train_preregistration": common.bind(args.train_preregistration), "train_decision": common.bind(args.train_decision),
            "corpus": common.bind(args.holdout_reserve), "engine": common.bind(args.teacher_engine),
            "weights": common.bind(args.teacher_weights), "base_engine": common.bind(args.base_engine),
            "base_ranking_auditor": common.bind(args.base_ranking_auditor), "reduced_engine": common.bind(args.reduced_engine),
            "reduced_ranking_auditor": common.bind(args.reduced_ranking_auditor), "base_candidate": candidates["base"],
            "reduced_candidate": candidates["reduced"],
        },
        "candidate_contract": train["candidate_contract"], "teacher_contract": train["teacher_contract"],
        "shallow_contract": train["shallow_contract"], "validation_contract": train["screen_contract"],
        "authorization": {"q27_authorized": False, "q20_authorized": False, "pass": "authorize Q27 only after both static and same-time conditions pass"},
        "tools": {
            "preparer": common.bind(Path(__file__).resolve()),
            "shallow_runner": common.bind(args.shallow_runner),
            "label_runner": common.bind(args.label_runner),
            "pair_builder": common.bind(args.pair_builder),
            # The screen is not part of the shared-label train runtime: bind
            # it here, before the sealed hold-out labels are generated.
            "screen_runner": common.bind(args.screen_runner),
        },
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("train-preregistration", "train-decision", "holdout-reserve", "teacher-engine", "teacher-weights", "base-engine", "base-ranking-auditor", "base-candidate", "reduced-engine", "reduced-ranking-auditor", "reduced-candidate", "shallow-runner", "label-runner", "pair-builder", "screen-runner", "output"):
        parser.add_argument(f"--{name}", type=Path, required=True)
    args = parser.parse_args()
    try:
        document = run(args)
        common.write_immutable(args.output, document)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    print(json.dumps({"status": document["status"], "parents": document["parents"]}, sort_keys=True))


if __name__ == "__main__":
    main()
