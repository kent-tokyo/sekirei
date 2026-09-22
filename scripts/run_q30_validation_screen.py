#!/usr/bin/env python3
"""Screen Q30's frozen 256/32 and 128/16 NNUEs on the sealed hold-out.

The experiment measures one trade-off only: whether the smaller evaluator
keeps the base model's static ranking closely enough while avoiding a
same-time regression against material.  It is a diagnostic screen, not a
candidate adoption or strength claim.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

import run_q21t_top_choice_pilot as q21t
import run_q21u_validation_screen as common


SCHEMA = "sekirei.q30-holdout-preregistration.v1"


def require(condition: bool, message: str) -> None:
    q21t.require(condition, message)


def bound(prereg: dict[str, Any], name: str, path: Path) -> None:
    require(prereg["inputs"][name]["sha256"] == q21t.sha256(path), f"Q30 {name} SHA mismatch")


def result_for_arm(
    *,
    arm: str,
    engine: Path,
    candidate: Path | None,
    position: dict[str, Any],
    scores: dict[str, int],
    teacher_engine: Path,
    teacher_weights: Path,
    timeout: float,
    time_ms: int,
) -> tuple[dict[str, Any], dict[str, Any] | None]:
    result = q21t.search_time(engine, position, candidate, time_ms)
    move = result["bestmove"]
    score = scores.get(move)
    fallback = None
    if score is None:
        fallback = common.fallback_teacher_score(
            teacher_engine, teacher_weights, position, move, timeout
        )
        score = fallback["score_cp"]
    top_score = max(scores.values())
    regret = top_score - score if abs(score) <= q21t.NORMAL_SCORE_ABS_MAX_CP else None
    return ({
        "parent_id": position["id"],
        "category": position["category"],
        "arm": arm,
        "result": result,
        "teacher_chosen_move_score_cp": score,
        "teacher_top_score_cp": top_score,
        "teacher_direct_top_regret_cp": regret,
    }, fallback)


def run(args: argparse.Namespace) -> dict[str, Any]:
    prereg = q21t.read(args.preregistration)
    pairs = q21t.read(args.validation_pairs)
    measurements = q21t.read(args.validation_measurements)
    require(
        prereg.get("schema") == SCHEMA
        and prereg.get("status") == "frozen_before_holdout_labels",
        "unexpected Q30 holdout preregistration",
    )
    for name, path in (
        ("engine", args.teacher_engine), ("weights", args.teacher_weights),
        ("base_engine", args.base_engine), ("base_ranking_auditor", args.base_ranking_auditor),
        ("reduced_engine", args.reduced_engine), ("reduced_ranking_auditor", args.reduced_ranking_auditor),
        ("base_candidate", args.base_candidate), ("reduced_candidate", args.reduced_candidate),
    ):
        bound(prereg, name, path)
    require(
        pairs.get("schema") == q21t.PAIR_SCHEMA
        and pairs.get("pair_selection") == "top-vs-rest",
        "Q30 validation pairs are not direct top-versus-rest pairs",
    )
    parent_ids = sorted({row["parent_id"] for row in pairs["pairs"]})
    require(parent_ids and len(parent_ids) <= prereg["parents"], "invalid Q30 validation parents")
    tables = common.teacher_tables(measurements, set(parent_ids))
    require(set(tables) == set(parent_ids), "Q30 ordinary-cp teacher table parent set mismatch")
    args.output_dir.mkdir(parents=True, exist_ok=True)

    plan = {
        "schema": "sekirei.q30-efficiency-same-time-plan.v1",
        "status": "frozen_before_same_time_measurement",
        "diagnostic_only": True,
        "strength_claim": False,
        "preregistration": q21t.bind(args.preregistration),
        "validation_pairs": q21t.bind(args.validation_pairs),
        "validation_measurements": q21t.bind(args.validation_measurements),
        "parent_ids": parent_ids,
        "contract": {
            "time_ms": args.time_ms, "repeats_per_arm": 2,
            "threads": 1, "spec_top_n": 0, "cold_process_per_search": True,
            "arms": {
                "base": "frozen 256/32 candidate", "reduced": "frozen 128/16 candidate",
                "material": "base binary without weights",
            },
            "repeat_order": ["base,reduced,material", "material,base,reduced"],
            "stability_requirement": "each arm must choose one identical move in both repeats",
            "mate_like_policy": "exclude terminal-or-mate-like fallback scores from ordinary-cp regret",
        },
        "tool_provenance": {
            "preregistered_sha256": prereg["tools"]["screen_runner"]["sha256"],
            "executed_sha256": q21t.sha256(Path(__file__).resolve()),
        },
    }
    plan_path = args.output_dir / "same-time-plan.json"
    q21t.atomic_write(plan_path, plan)

    audits = {
        "base": q21t.run_static_audit(args.base_ranking_auditor, args.validation_pairs, args.base_candidate),
        "reduced": q21t.run_static_audit(args.reduced_ranking_auditor, args.validation_pairs, args.reduced_candidate),
    }
    for name, audit in audits.items():
        q21t.atomic_write(args.output_dir / f"static-{name}.json", audit)
    static = {name: q21t.static_metrics(audit) for name, audit in audits.items()}
    base_mean = static["base"]["mean_direct_top_regret_cp"]
    static_conditions = {
        "reduced_mean_regret_within_10pct_of_base": (
            static["reduced"]["mean_direct_top_regret_cp"] <= base_mean * 1.10
        ),
        "reduced_major_regrets_not_above_base": (
            static["reduced"]["major_regrets_ge_300cp"] <= static["base"]["major_regrets_ge_300cp"]
        ),
    }

    rows: list[dict[str, Any]] = []
    fallbacks: list[dict[str, Any]] = []
    arm_inputs = {
        "base": (args.base_engine, args.base_candidate),
        "reduced": (args.reduced_engine, args.reduced_candidate),
        "material": (args.base_engine, None),
    }
    for repetition, order in enumerate((("base", "reduced", "material"), ("material", "base", "reduced"))):
        for parent_id in parent_ids:
            table = tables[parent_id]
            for arm in order:
                row, fallback = result_for_arm(
                    arm=arm, engine=arm_inputs[arm][0], candidate=arm_inputs[arm][1],
                    position=table["position"], scores=table["scores"], teacher_engine=args.teacher_engine,
                    teacher_weights=args.teacher_weights,
                    timeout=prereg["teacher_contract"]["timeout_seconds_per_search"], time_ms=args.time_ms,
                )
                row["repetition"] = repetition
                rows.append(row)
                if fallback is not None:
                    fallbacks.append({"parent_id": parent_id, "arm": arm, "move": row["result"]["bestmove"], **fallback})
    same_time = {arm: q21t.same_time_metrics(rows, arm) for arm in arm_inputs}
    same_time_conditions = {
        "all_arms_stable": all(value["stable_parents"] == len(parent_ids) for value in same_time.values()),
        "all_arms_comparable": all(value["comparable_parents"] == len(parent_ids) for value in same_time.values()),
        "reduced_mean_regret_not_above_material": (
            same_time["reduced"]["mean_direct_top_regret_cp"] <= same_time["material"]["mean_direct_top_regret_cp"]
        ),
        "reduced_major_regrets_not_above_material": (
            same_time["reduced"]["major_regrets_ge_300cp"] <= same_time["material"]["major_regrets_ge_300cp"]
        ),
        "reduced_top1_matches_not_below_material": (
            same_time["reduced"]["top1_matches"] >= same_time["material"]["top1_matches"]
        ),
    }
    q21t.atomic_write(args.output_dir / "same-time-measurements.json", {
        "schema": "sekirei.q30-efficiency-same-time-measurements.v1", "status": "complete",
        "diagnostic_only": True, "strength_claim": False, "plan": q21t.bind(plan_path),
        "rows": rows, "fallback_depth7_labels": fallbacks,
    })
    static_pass, same_time_pass = all(static_conditions.values()), all(same_time_conditions.values())
    passed = static_pass and same_time_pass
    decision = {
        "schema": "sekirei.q30-efficiency-validation-decision.v1",
        "status": "pass" if passed else "fail", "diagnostic_only": True, "strength_claim": False,
        "experiment_complete": True, "candidate_adopted": False,
        "q27_authorized": passed, "q20_authorized": False,
        "static": {**static, "conditions": static_conditions, "pass": static_pass},
        "same_time": {**same_time, "conditions": same_time_conditions, "pass": same_time_pass},
        "next_action": (
            "run the preregistered Q27 candidate-versus-material screen for the reduced arm"
            if passed else "reject the Q30 reduced arm; do not run Q27 or Q20"
        ),
        "artifacts": {
            name: q21t.bind(path) for name, path in {
                "preregistration": args.preregistration, "validation_pairs": args.validation_pairs,
                "validation_measurements": args.validation_measurements, "same_time_plan": plan_path,
                "base_candidate": args.base_candidate, "reduced_candidate": args.reduced_candidate,
                "base_static_audit": args.output_dir / "static-base.json",
                "reduced_static_audit": args.output_dir / "static-reduced.json",
                "same_time_measurements": args.output_dir / "same-time-measurements.json",
                "runner": Path(__file__).resolve(),
            }.items()
        },
    }
    q21t.atomic_write(args.output_dir / "decision.json", decision)
    report = [
        "# Q30 efficiency frontier validation", "",
        "This is a sealed-holdout diagnostic, not a strength claim.", "",
        f"- Status: `{decision['status']}`.",
        f"- Static base/reduced regret: {base_mean:.3f} / {static['reduced']['mean_direct_top_regret_cp']:.3f} cp.",
        f"- Same-time base/reduced/material regret: {same_time['base']['mean_direct_top_regret_cp']:.3f} / {same_time['reduced']['mean_direct_top_regret_cp']:.3f} / {same_time['material']['mean_direct_top_regret_cp']:.3f} cp.",
        f"- Q27 authorized: {decision['q27_authorized']}.", "- Q20 remains unauthorized.", "",
    ]
    (args.output_dir / "report.md").write_text("\n".join(report), encoding="utf-8")
    return decision


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in (
        "preregistration", "validation-pairs", "validation-measurements", "teacher-engine",
        "teacher-weights", "base-engine", "base-ranking-auditor", "base-candidate",
        "reduced-engine", "reduced-ranking-auditor", "reduced-candidate", "output-dir",
    ):
        parser.add_argument(f"--{name}", type=Path, required=True)
    parser.add_argument("--time-ms", type=int, default=1_000)
    args = parser.parse_args()
    try:
        decision = run(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError, RuntimeError) as error:
        parser.error(str(error))
    print(json.dumps({"status": decision["status"], "q27_authorized": decision["q27_authorized"]}, sort_keys=True))
    return 0 if decision["status"] == "pass" else 1


if __name__ == "__main__":
    raise SystemExit(main())
