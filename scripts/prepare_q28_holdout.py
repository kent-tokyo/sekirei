#!/usr/bin/env python3
"""Freeze Q28's trained candidate before opening its reserved hold-out."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

import prepare_q28_training as common


PHASES = {
    "sekirei.q28-train-preregistration.v1": ("q28", "sekirei.q28-holdout-preregistration.v1", 18),
    "sekirei.q29-train-preregistration.v1": ("q29", "sekirei.q29-holdout-preregistration.v1", 36),
}


def unique(values: set[Any], expected: int, name: str, phase: str) -> None:
    common.require(None not in values and len(values) == expected, f"{phase.upper()} holdout {name} is not unique")


def run(args: argparse.Namespace) -> dict[str, Any]:
    train_prereg = common.read(args.train_preregistration)
    decision = common.read(args.train_decision)
    holdout = common.read(args.holdout_reserve)
    phase_spec = PHASES.get(train_prereg.get("schema"))
    common.require(phase_spec is not None, "unexpected train preregistration")
    phase, schema, parents = phase_spec
    common.require(
        train_prereg.get("status") == "frozen_before_train_labels",
        f"unexpected {phase.upper()} train preregistration",
    )
    common.require(
        decision.get("schema") == f"sekirei.{phase}-train-decision.v1"
        and decision.get("status") == "candidate_frozen"
        and decision.get("holdout_inspected") is False
        and decision.get("holdout_labels_generated") is False,
        f"{phase.upper()} candidate was not frozen before holdout",
    )
    common.require(
        train_prereg["inputs"]["holdout_reserve_sealed"]["sha256"] == common.sha256(args.holdout_reserve),
        f"{phase.upper()} holdout membership SHA mismatch",
    )
    rows = common.positions(holdout, f"sekirei.{phase}-holdout-reserve.v1", parents)
    unique({row.get("source", {}).get("source_key") for row in rows}, parents, "CSA source", phase)
    unique({row.get("source", {}).get("derived_group") for row in rows}, parents, "derived group", phase)
    candidate = common.bind(args.candidate)
    common.require(
        decision["candidate"]["artifact"]["sha256"] == candidate["sha256"],
        f"{phase.upper()} candidate SHA differs from the train-only decision",
    )
    for name, path in (("engine", args.engine), ("weights", args.teacher_weights), ("ranking_auditor", args.ranking_auditor)):
        common.require(train_prereg["inputs"][name]["sha256"] == common.sha256(path), f"{phase.upper()} {name} SHA mismatch")
    document = {
        "schema": schema,
        "status": "frozen_before_holdout_labels",
        "diagnostic_only": True,
        "strength_claim": False,
        "single_factor": train_prereg["single_factor"],
        "parents": parents,
        "strata": sorted({row["category"] for row in rows}),
        "inputs": {
            "train_preregistration": common.bind(args.train_preregistration),
            "train_decision": common.bind(args.train_decision),
            "corpus": common.bind(args.holdout_reserve),
            "engine": common.bind(args.engine),
            "weights": common.bind(args.teacher_weights),
            "ranking_auditor": common.bind(args.ranking_auditor),
            "candidate": candidate,
        },
        "candidate_contract": train_prereg["candidate_contract"],
        "teacher_contract": train_prereg["teacher_contract"],
        "shallow_contract": train_prereg["shallow_contract"],
        "validation_contract": {
            "fresh_holdout": True,
            "selection_used_holdout": False,
            "baseline": "initial fixed-T checkpoint",
            "minimum_mean_direct_top_regret_reduction": 0.10,
            "top1_matches_must_not_decrease": True,
            "major_regrets_ge_300cp_must_not_increase": True,
            "same_time_candidate_must_not_trail_material": True,
            "time_ms": 1000,
            "repeats_per_arm": 2,
        },
        "authorization": {
            "pass": "authorize only the preregistered Q27 32-game candidate-versus-material screen",
            "fail": f"reject {phase.upper()}; do not run Q27 or Q20",
            "q27_authorized": False,
            "q20_authorized": False,
        },
        "tools": {
            "preparer": common.bind(Path(__file__).resolve()),
            "shallow_runner": common.bind(args.shallow_runner),
            "label_runner": common.bind(args.label_runner),
            "pair_builder": common.bind(args.pair_builder),
            "screen_runner": common.bind(args.screen_runner),
        },
    }
    common.write_immutable(args.output, document)
    return document


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--train-preregistration", type=Path, required=True)
    parser.add_argument("--train-decision", type=Path, required=True)
    parser.add_argument("--holdout-reserve", type=Path, required=True)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--teacher-weights", type=Path, required=True)
    parser.add_argument("--ranking-auditor", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--shallow-runner", type=Path, required=True)
    parser.add_argument("--label-runner", type=Path, required=True)
    parser.add_argument("--pair-builder", type=Path, required=True)
    parser.add_argument("--screen-runner", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        document = run(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    print(json.dumps({"status": document["status"], "parents": document["parents"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
