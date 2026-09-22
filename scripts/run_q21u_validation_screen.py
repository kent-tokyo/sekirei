#!/usr/bin/env python3
"""Run Q21u's frozen-candidate static and same-time validation screens."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
from statistics import mean
from typing import Any

import run_q21t_top_choice_pilot as q21t


PREREG_SCHEMA = "sekirei.q21u-listwise-validation-preregistration.v1"
Q21X_PREREG_SCHEMA = "sekirei.q21x-holdout-preregistration.v1"
Q28_PREREG_SCHEMA = "sekirei.q28-holdout-preregistration.v1"
Q29_PREREG_SCHEMA = "sekirei.q29-holdout-preregistration.v1"


def phase_contract(prereg: dict[str, Any]) -> tuple[str, str]:
    schema = prereg.get("schema")
    if schema == PREREG_SCHEMA:
        return "q21u", "frozen_before_depth7_labels_and_validation_screen"
    if schema == Q21X_PREREG_SCHEMA:
        return "q21x", "frozen_before_holdout_labels"
    if schema == Q28_PREREG_SCHEMA:
        return "q28", "frozen_before_holdout_labels"
    if schema == Q29_PREREG_SCHEMA:
        return "q29", "frozen_before_holdout_labels"
    raise ValueError("unexpected validation preregistration")


def bind(path: Path) -> dict[str, str]:
    return q21t.bind(path)


def require(condition: bool, message: str) -> None:
    q21t.require(condition, message)


def same_time_metrics(rows: list[dict[str, Any]], arm: str) -> dict[str, Any]:
    return q21t.same_time_metrics(rows, arm)


def exact_depth7_or_terminal(result: dict[str, Any]) -> bool:
    if q21t.exact_depth(result):
        return True
    score = result.get("score_cp")
    return (
        result.get("completion") == "search_completed"
        and result.get("completed_bound") == "exact"
        and result.get("completed_iteration_valid") in {True, "true"}
        and result.get("aborted") in {False, "false"}
        and result.get("abort_reason") == "none"
        and result.get("pv_legal") is True
        and result.get("pv_replay_preserves_input") is True
        and result.get("history_matches_expected") in {True, "true"}
        and isinstance(result.get("depth"), int)
        and 0 < result["depth"] < 7
        and isinstance(score, int)
        and abs(score) > q21t.NORMAL_SCORE_ABS_MAX_CP
    )


def fallback_teacher_score(
    binary: Path,
    teacher: Path,
    position: dict[str, Any],
    move: str,
    timeout: float,
) -> dict[str, Any]:
    os.environ["RAYON_NUM_THREADS"] = "1"
    repeats = [
        q21t.run_position(
            binary,
            position["initial_sfen"],
            1,
            timeout,
            teacher,
            root_move=move,
            max_depth=7,
            history_moves_usi=position["history_before_usi"],
            expected_sfen=position["sfen"],
            nnue_output="residual-material",
        )
        for _ in range(2)
    ]
    require(
        all(exact_depth7_or_terminal(result) for result in repeats),
        f"{position['id']}/{move}: incomplete fallback label",
    )
    signature = lambda result: (
        result["bestmove"],
        result["score_cp"],
        result["depth"],
        result["completed_bound"],
    )
    require(
        signature(repeats[0]) == signature(repeats[1]),
        f"{position['id']}/{move}: fallback A/A mismatch",
    )
    return {
        "score_cp": repeats[0]["score_cp"],
        "score_kind": (
            "ordinary_depth7"
            if abs(repeats[0]["score_cp"]) <= q21t.NORMAL_SCORE_ABS_MAX_CP
            else "terminal_or_mate_like"
        ),
        "repeats": repeats,
    }


def teacher_tables(
    measurements: dict[str, Any], parent_ids: set[str]
) -> dict[str, dict[str, Any]]:
    tables = {}
    for row in measurements.get("rows", []):
        if row.get("id") not in parent_ids:
            continue
        fixed = row.get("fixed", [])
        require(len(fixed) == 2, f"{row.get('id')}: depth-7 repeat count mismatch")
        scores = {}
        for move in row.get("candidate_moves", []):
            left, right = fixed[0][move], fixed[1][move]
            require(
                exact_depth7_or_terminal(left) and exact_depth7_or_terminal(right),
                f"{row['id']}/{move}: incomplete label",
            )
            require(
                (left["bestmove"], left["score_cp"], left["depth"])
                == (right["bestmove"], right["score_cp"], right["depth"]),
                f"{row['id']}/{move}: label A/A mismatch",
            )
            if q21t.exact_depth(left) and abs(left["score_cp"]) <= q21t.NORMAL_SCORE_ABS_MAX_CP:
                scores[move] = left["score_cp"]
        require(len(scores) >= 2, f"{row['id']}: fewer than two ordinary depth-7 scores")
        tables[row["id"]] = {"position": row, "scores": scores}
    return tables


def run(args: argparse.Namespace) -> dict[str, Any]:
    prereg = q21t.read(args.preregistration)
    pairs = q21t.read(args.validation_pairs)
    phase, expected_status = phase_contract(prereg)
    require(
        prereg.get("status") == expected_status,
        f"unexpected {phase} validation preregistration status",
    )
    for name, path in (
        ("engine", args.engine),
        ("ranking_auditor", args.ranking_auditor),
        ("weights", args.baseline),
        ("candidate", args.candidate),
    ):
        require(prereg["inputs"][name]["sha256"] == q21t.sha256(path), f"{name} SHA mismatch")
    require(
        pairs.get("schema") == q21t.PAIR_SCHEMA
        and pairs.get("pair_selection") == "top-vs-rest",
        "validation pairs are not direct top-versus-rest pairs",
    )
    parent_ids = sorted({row["parent_id"] for row in pairs["pairs"]})
    require(len(parent_ids) <= prereg["parents"], "validation parent count exceeds preregistration")
    args.output_dir.mkdir(parents=True, exist_ok=True)

    plan = {
        "schema": f"sekirei.{phase}-same-time-plan.v1",
        "status": "frozen_before_same_time_measurement",
        "diagnostic_only": True,
        "strength_claim": False,
        "preregistration": bind(args.preregistration),
        "validation_pairs": bind(args.validation_pairs),
        "validation_measurements": bind(args.validation_measurements),
        "candidate": bind(args.candidate),
        "parent_ids": parent_ids,
        "excluded_parents": prereg["parents"] - len(parent_ids),
        "contract": {
            "time_ms": args.time_ms,
            "repeats_per_arm": 2,
            "threads": 1,
            "spec_top_n": 0,
            "cold_process_per_search": True,
            "arms": {"candidate": f"{phase.upper()} frozen candidate", "material": "no weights"},
            "repeat_order": ["candidate,material", "material,candidate"],
            "stability_requirement": "each arm must choose one identical move in both repeats",
            "mate_like_fallback": (
                "an exact deterministic mate-like result completed before depth 7 is recorded but "
                "excluded from ordinary-cp regret; an incomparable parent makes the screen fail"
            ),
        },
        "tool_provenance": {
            "preregistered_sha256": prereg["tools"]["screen_runner"]["sha256"],
            "executed_sha256": q21t.sha256(Path(__file__).resolve()),
            "contract_preserving_fix_after_preregistration": (
                prereg["tools"]["screen_runner"]["sha256"]
                != q21t.sha256(Path(__file__).resolve())
            ),
            "fix_scope": (
                "record deterministic exact mate-like fallback scores completed before depth 7 as "
                "incomparable instead of aborting the frozen screen"
            ),
        },
    }
    plan_path = args.output_dir / "same-time-plan.json"
    q21t.atomic_write(plan_path, plan)

    audits = {
        "baseline": q21t.run_static_audit(args.ranking_auditor, args.validation_pairs, args.baseline),
        "candidate": q21t.run_static_audit(args.ranking_auditor, args.validation_pairs, args.candidate),
    }
    for name, audit in audits.items():
        q21t.atomic_write(args.output_dir / f"static-{name}.json", audit)
    static = {name: q21t.static_metrics(audit) for name, audit in audits.items()}
    baseline_mean = static["baseline"]["mean_direct_top_regret_cp"]
    reduction = (
        (baseline_mean - static["candidate"]["mean_direct_top_regret_cp"]) / baseline_mean
        if baseline_mean > 0
        else 0.0
    )
    static_conditions = {
        "mean_direct_top_regret_reduction_at_least_10pct": reduction >= 0.10,
        "top1_matches_not_decreased": static["candidate"]["top1_matches"] >= static["baseline"]["top1_matches"],
        "major_regrets_not_increased": (
            static["candidate"]["major_regrets_ge_300cp"]
            <= static["baseline"]["major_regrets_ge_300cp"]
        ),
    }

    measurements = q21t.read(args.validation_measurements)
    tables = teacher_tables(measurements, set(parent_ids))
    require(set(tables) == set(parent_ids), "ordinary-cp teacher table parent set mismatch")
    rows = []
    fallback_labels = []
    orders = (("candidate", "material"), ("material", "candidate"))
    for repetition, order in enumerate(orders):
        for parent_id in parent_ids:
            table = tables[parent_id]
            position = table["position"]
            top_score = max(table["scores"].values())
            for arm in order:
                result = q21t.search_time(
                    args.engine,
                    position,
                    args.candidate if arm == "candidate" else None,
                    args.time_ms,
                )
                move = result["bestmove"]
                score = table["scores"].get(move)
                if score is None:
                    label = fallback_teacher_score(
                        args.engine,
                        args.baseline,
                        position,
                        move,
                        prereg["teacher_contract"]["timeout_seconds_per_search"],
                    )
                    score = label["score_cp"]
                    fallback_labels.append({"parent_id": parent_id, "arm": arm, "move": move, **label})
                regret = top_score - score if abs(score) <= q21t.NORMAL_SCORE_ABS_MAX_CP else None
                rows.append(
                    {
                        "parent_id": parent_id,
                        "category": position["category"],
                        "repetition": repetition,
                        "arm": arm,
                        "result": result,
                        "teacher_chosen_move_score_cp": score,
                        "teacher_top_score_cp": top_score,
                        "teacher_direct_top_regret_cp": regret,
                    }
                )
    same_time = {arm: same_time_metrics(rows, arm) for arm in ("candidate", "material")}
    same_time_conditions = {
        "all_candidate_bestmoves_stable": same_time["candidate"]["stable_parents"] == len(parent_ids),
        "all_material_bestmoves_stable": same_time["material"]["stable_parents"] == len(parent_ids),
        "all_parents_comparable": all(
            same_time[arm]["comparable_parents"] == len(parent_ids)
            for arm in ("candidate", "material")
        ),
        "candidate_mean_regret_not_above_material": (
            same_time["candidate"]["mean_direct_top_regret_cp"]
            <= same_time["material"]["mean_direct_top_regret_cp"]
        ),
        "candidate_major_regrets_not_above_material": (
            same_time["candidate"]["major_regrets_ge_300cp"]
            <= same_time["material"]["major_regrets_ge_300cp"]
        ),
        "candidate_top1_matches_not_below_material": (
            same_time["candidate"]["top1_matches"] >= same_time["material"]["top1_matches"]
        ),
    }
    measurements_document = {
        "schema": f"sekirei.{phase}-same-time-measurements.v1",
        "status": "complete",
        "diagnostic_only": True,
        "strength_claim": False,
        "plan": bind(plan_path),
        "rows": rows,
        "fallback_depth7_labels": fallback_labels,
    }
    q21t.atomic_write(args.output_dir / "same-time-measurements.json", measurements_document)

    static_pass = all(static_conditions.values())
    same_time_pass = all(same_time_conditions.values())
    passed = static_pass and same_time_pass
    decision = {
        "schema": f"sekirei.{phase}-listwise-validation-decision.v1",
        "status": "pass" if passed else "fail",
        "diagnostic_only": True,
        "strength_claim": False,
        "experiment_complete": True,
        "candidate_adopted": False,
        "development_match_authorized": passed if phase == "q21u" else False,
        "q21y_authorized": passed if phase == "q21x" else False,
        "q27_authorized": passed if phase in {"q28", "q29"} else False,
        "q20_authorized": False,
        "static": {
            **static,
            "candidate_mean_regret_reduction": reduction,
            "conditions": static_conditions,
            "pass": static_pass,
        },
        "same_time": {**same_time, "conditions": same_time_conditions, "pass": same_time_pass},
        "next_action": (
            (
                "continue to Q21y capacity-only comparison"
                if phase == "q21x"
                else ("run the preregistered Q27 32-game candidate-versus-material screen" if phase in {"q28", "q29"} else "run only the preregistered 32-game development match")
            )
            if passed
            else f"reject the {phase.upper()} candidate; do not run the development match or Q20"
        ),
        "artifacts": {
            name: bind(path)
            for name, path in {
                "preregistration": args.preregistration,
                "same_time_plan": plan_path,
                "validation_pairs": args.validation_pairs,
                "validation_measurements": args.validation_measurements,
                "baseline_static_audit": args.output_dir / "static-baseline.json",
                "candidate": args.candidate,
                "candidate_static_audit": args.output_dir / "static-candidate.json",
                "same_time_measurements": args.output_dir / "same-time-measurements.json",
                "runner": Path(__file__).resolve(),
            }.items()
        },
    }
    q21t.atomic_write(args.output_dir / "decision.json", decision)
    report = [
        f"# {phase.upper()} listwise validation screen",
        "",
        "This is a diagnostic screen on a fresh holdout, not a strength result.",
        "",
        f"- Status: `{decision['status']}`.",
        f"- Static direct top regret: {baseline_mean:.3f} -> "
        f"{static['candidate']['mean_direct_top_regret_cp']:.3f} cp "
        f"({reduction * 100:.2f}% reduction).",
        f"- Static top-1 matches: {static['baseline']['top1_matches']} -> "
        f"{static['candidate']['top1_matches']} / {static['candidate']['parents']}.",
        f"- Same-time mean regret, candidate/material: "
        f"{same_time['candidate']['mean_direct_top_regret_cp']:.3f} / "
        f"{same_time['material']['mean_direct_top_regret_cp']:.3f} cp.",
        f"- Development match authorized: {decision['development_match_authorized']}.",
        f"- Q27 authorized: {decision['q27_authorized']}.",
        "- Q20 remains unauthorized.",
        "",
    ]
    (args.output_dir / "report.md").write_text("\n".join(report), encoding="utf-8")
    return decision


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preregistration", type=Path, required=True)
    parser.add_argument("--validation-pairs", type=Path, required=True)
    parser.add_argument("--validation-measurements", type=Path, required=True)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--ranking-auditor", type=Path, required=True)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--time-ms", type=int, default=1_000)
    args = parser.parse_args()
    try:
        decision = run(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError, RuntimeError) as error:
        parser.error(str(error))
    print(
        json.dumps(
            {
                "status": decision["status"],
                "static_pass": decision["static"]["pass"],
                "same_time_pass": decision["same_time"]["pass"],
                "development_match_authorized": decision["development_match_authorized"],
            },
            sort_keys=True,
        )
    )
    return 0 if decision["status"] == "pass" else 1


if __name__ == "__main__":
    raise SystemExit(main())
