#!/usr/bin/env python3
"""Run and decide Q21t's direct top-choice 1-seed pilot.

The static screen uses direct teacher-top regret.  The same-time screen then
checks whether the trained evaluator beats material under the practical NNUE
cost.  This remains a diagnostic pilot and never makes a strength claim.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
from pathlib import Path
from statistics import mean
from typing import Any

from run_core_floodgate_diagnostic import run_position
from run_q21j_failure_audit import parse_profile


PREREG_SCHEMA = "sekirei.q21t-top-choice-pilot-preregistration.v1"
PAIR_SCHEMA = "sekirei.root-rank-pairs.v1"
AUDIT_SCHEMA = "sekirei.root-rank-pair-audit.v1"
NORMAL_SCORE_ABS_MAX_CP = 10_000
MAJOR_REGRET_CP = 300


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def bind(path: Path) -> dict[str, str]:
    if not path.is_file():
        raise ValueError(f"missing input: {path}")
    return {"path": str(path), "sha256": sha256(path)}


def read(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def atomic_write(path: Path, document: dict[str, Any]) -> None:
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(
        json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    os.replace(temporary, path)


def run_json(command: list[str], timeout: float) -> dict[str, Any]:
    completed = subprocess.run(
        command,
        text=True,
        capture_output=True,
        check=False,
        timeout=timeout,
        env={**os.environ, "RAYON_NUM_THREADS": "1"},
    )
    if completed.returncode:
        raise RuntimeError(
            f"command failed ({completed.returncode}): {' '.join(command)}\n"
            f"{completed.stderr[-3000:]}"
        )
    return json.loads(completed.stdout)


def run_static_audit(auditor: Path, pairs: Path, weights: Path) -> dict[str, Any]:
    report = run_json(
        [str(auditor), "--pairs", str(pairs), "--weights", str(weights)],
        timeout=60.0,
    )
    require(
        report.get("schema") == AUDIT_SCHEMA
        and report.get("source", {}).get("pair_selection") == "top-vs-rest",
        "static audit did not use direct top-versus-rest pairs",
    )
    return report


def static_metrics(audit: dict[str, Any]) -> dict[str, Any]:
    diagnostic = audit.get("model_diagnostic", {})
    rows = diagnostic.get("parent_diagnostics", [])
    require(rows and len(rows) == audit.get("parent_positions"), "static audit is incomplete")
    return {
        "parents": len(rows),
        "mean_direct_top_regret_cp": float(diagnostic["mean_parent_rank_loss_cp"]),
        "top1_matches": sum(row["teacher_rank_loss_cp"] == 0 for row in rows),
        "major_regrets_ge_300cp": sum(
            row["teacher_rank_loss_cp"] >= MAJOR_REGRET_CP for row in rows
        ),
        "teacher_preferred_ordering_rate": diagnostic["teacher_preferred_ordering_rate"],
    }


def search_time(
    binary: Path,
    position: dict[str, Any],
    weights: Path | None,
    time_ms: int,
) -> dict[str, Any]:
    command = [
        str(binary),
        "--profile-cost",
        "--sfen",
        position["initial_sfen"],
        "--moves",
        " ".join(position["history_before_usi"]),
        "--expected-sfen",
        position["sfen"],
        "--time-ms",
        str(time_ms),
    ]
    if weights is not None:
        command.extend(
            (
                "--weights",
                str(weights),
                "--nnue-output",
                "residual-material",
                "--nnue-residual-scale-permille",
                "1000",
            )
        )
    completed = subprocess.run(
        command,
        text=True,
        capture_output=True,
        check=False,
        timeout=time_ms / 1000 + 30.0,
        env={**os.environ, "RAYON_NUM_THREADS": "1"},
    )
    if completed.returncode:
        raise RuntimeError(f"same-time diagnostic failed: {completed.stderr[-3000:]}")
    return parse_profile(completed.stdout)


def exact_depth(result: dict[str, Any], depth: int = 7) -> bool:
    return (
        result.get("depth") == depth
        and result.get("completed_bound") == "exact"
        and result.get("completed_iteration_valid") in {True, "true"}
        and result.get("aborted") in {False, "false"}
        and result.get("pv_legal") is True
        and result.get("pv_replay_preserves_input") is True
        and result.get("history_matches_expected") in {True, "true"}
        and isinstance(result.get("score_cp"), int)
    )


def depth7_score(
    binary: Path,
    teacher: Path,
    position: dict[str, Any],
    move: str,
    timeout: float,
) -> dict[str, Any]:
    os.environ["RAYON_NUM_THREADS"] = "1"
    repeats = [
        run_position(
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
    require(all(exact_depth(row) for row in repeats), f"{position['id']}/{move}: incomplete depth-7 label")
    signature = lambda row: (row["bestmove"], row["score_cp"], row["depth"], row["completed_bound"])
    require(signature(repeats[0]) == signature(repeats[1]), f"{position['id']}/{move}: depth-7 A/A mismatch")
    return {"score_cp": repeats[0]["score_cp"], "repeats": repeats}


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
            require(exact_depth(left) and exact_depth(right), f"{row['id']}/{move}: incomplete label")
            require(
                (left["bestmove"], left["score_cp"]) == (right["bestmove"], right["score_cp"]),
                f"{row['id']}/{move}: label A/A mismatch",
            )
            if abs(left["score_cp"]) <= NORMAL_SCORE_ABS_MAX_CP:
                scores[move] = left["score_cp"]
        tables[row["id"]] = {"position": row, "scores": scores}
    return tables


def same_time_metrics(rows: list[dict[str, Any]], arm: str) -> dict[str, Any]:
    selected = [row for row in rows if row["arm"] == arm]
    by_parent: dict[str, list[dict[str, Any]]] = {}
    for row in selected:
        by_parent.setdefault(row["parent_id"], []).append(row)
    parent_rows = []
    for parent_id, repeats in sorted(by_parent.items()):
        require(len(repeats) == 2, f"{arm}/{parent_id}: expected two repeats")
        moves = {row["result"]["bestmove"] for row in repeats}
        stable = len(moves) == 1
        regrets = [row.get("teacher_direct_top_regret_cp") for row in repeats]
        comparable = stable and all(isinstance(value, int) for value in regrets)
        parent_rows.append(
            {
                "parent_id": parent_id,
                "stable_bestmove": stable,
                "bestmoves": [row["result"]["bestmove"] for row in repeats],
                "comparable": comparable,
                "teacher_direct_top_regret_cp": regrets[0] if comparable else None,
            }
        )
    comparable = [row for row in parent_rows if row["comparable"]]
    return {
        "parents": len(parent_rows),
        "stable_parents": sum(row["stable_bestmove"] for row in parent_rows),
        "comparable_parents": len(comparable),
        "mean_direct_top_regret_cp": mean(
            row["teacher_direct_top_regret_cp"] for row in comparable
        ) if comparable else None,
        "top1_matches": sum(row["teacher_direct_top_regret_cp"] == 0 for row in comparable),
        "major_regrets_ge_300cp": sum(
            row["teacher_direct_top_regret_cp"] >= MAJOR_REGRET_CP for row in comparable
        ),
        "parent_diagnostics": parent_rows,
    }


def validate_inputs(args: argparse.Namespace) -> tuple[dict[str, Any], dict[str, Any]]:
    prereg = read(args.preregistration)
    pairs = read(args.validation_pairs)
    require(
        prereg.get("schema") == PREREG_SCHEMA
        and prereg.get("status") == "frozen_before_depth7_labels_and_candidate_training",
        "unexpected Q21t preregistration",
    )
    for name, path in (
        ("engine", args.engine),
        ("ranking_auditor", args.ranking_auditor),
        ("trainer", args.trainer),
        ("weights", args.baseline),
        ("train_pairs", args.train_pairs),
    ):
        require(prereg["inputs"][name]["sha256"] == sha256(path), f"{name} SHA mismatch")
    require(
        pairs.get("schema") == PAIR_SCHEMA
        and pairs.get("pair_selection") == "top-vs-rest",
        "validation pairs are not direct top-versus-rest pairs",
    )
    return prereg, pairs


def freeze_same_time_plan(
    args: argparse.Namespace,
    prereg: dict[str, Any],
    pairs: dict[str, Any],
) -> dict[str, Any]:
    parent_ids = sorted({row["parent_id"] for row in pairs["pairs"]})
    document = {
        "schema": "sekirei.q21t-same-time-plan.v1",
        "status": "frozen_before_candidate_training_and_same_time_measurement",
        "diagnostic_only": True,
        "strength_claim": False,
        "preregistration": bind(args.preregistration),
        "validation_pairs": bind(args.validation_pairs),
        "validation_measurements": bind(args.validation_measurements),
        "parent_ids": parent_ids,
        "excluded_parents": prereg["parents"] - len(parent_ids),
        "contract": {
            "time_ms": args.time_ms,
            "repeats_per_arm": 2,
            "threads": 1,
            "spec_top_n": 0,
            "cold_process_per_search": True,
            "arms": {
                "candidate": "Q21t candidate at residual-material scale=1000",
                "material": "no weights",
            },
            "repeat_order": ["candidate,material", "material,candidate"],
            "stability_requirement": "each arm must choose one identical move in both repeats",
            "regret": "depth-7 teacher-top score minus chosen-move score on ordinary-cp parents",
            "outside_union_contingency": (
                "run the chosen move twice as a forced-root depth-7 search and require exact A/A agreement"
            ),
        },
        "candidate_output": str(args.candidate),
    }
    path = args.output_dir / "same-time-plan.json"
    encoded = json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    if path.exists():
        require(path.read_text(encoding="utf-8") == encoded, "existing same-time plan differs")
    else:
        path.write_text(encoded, encoding="utf-8")
    return document


def train_candidate(args: argparse.Namespace, prereg: dict[str, Any]) -> None:
    contract = prereg["training_contract"]
    metadata = args.candidate.with_suffix(".ranking.json")
    if args.candidate.exists() or metadata.exists():
        require(args.candidate.is_file() and metadata.is_file(), "partial candidate output exists")
        existing = read(metadata)
        require(
            existing.get("pairs_path") == str(args.train_pairs)
            and existing.get("epochs") == contract["epochs"]
            and existing.get("ranking_parent_balanced") is True,
            "existing candidate metadata differs from Q21t contract",
        )
        return
    command = [
        str(args.trainer),
        "--ranking-pairs",
        str(args.train_pairs),
        "--init-weights",
        str(args.baseline),
        "--output",
        str(args.candidate),
        "--ranking-epochs",
        str(contract["epochs"]),
        "--ranking-batch-pairs",
        "1",
        "--ranking-parent-balanced",
        "--lr",
        str(contract["learning_rate"]),
        "--nnue-output",
        contract["nnue_output"],
        "--seed",
        str(contract["seed"]),
    ]
    completed = subprocess.run(command, text=True, capture_output=True, check=False, timeout=600)
    if completed.returncode:
        raise RuntimeError(f"Q21t training failed: {completed.stderr[-4000:]}")


def run(args: argparse.Namespace) -> dict[str, Any]:
    prereg, pairs = validate_inputs(args)
    args.output_dir.mkdir(parents=True, exist_ok=True)
    plan = freeze_same_time_plan(args, prereg, pairs)

    audits = {}
    for name, weights in (("baseline", args.baseline), ("q21p", args.old_candidate)):
        audits[name] = run_static_audit(args.ranking_auditor, args.validation_pairs, weights)
        atomic_write(args.output_dir / f"static-{name}.json", audits[name])

    train_candidate(args, prereg)
    audits["candidate"] = run_static_audit(
        args.ranking_auditor, args.validation_pairs, args.candidate
    )
    atomic_write(args.output_dir / "static-candidate.json", audits["candidate"])
    static = {name: static_metrics(report) for name, report in audits.items()}
    baseline_mean = static["baseline"]["mean_direct_top_regret_cp"]
    reduction = (
        (baseline_mean - static["candidate"]["mean_direct_top_regret_cp"]) / baseline_mean
        if baseline_mean > 0
        else 0.0
    )
    static_conditions = {
        "mean_direct_top_regret_reduction_at_least_10pct": reduction >= 0.10,
        "top1_matches_not_decreased": (
            static["candidate"]["top1_matches"] >= static["baseline"]["top1_matches"]
        ),
        "major_regrets_not_increased": (
            static["candidate"]["major_regrets_ge_300cp"]
            <= static["baseline"]["major_regrets_ge_300cp"]
        ),
    }

    parent_ids = set(plan["parent_ids"])
    measurements = read(args.validation_measurements)
    tables = teacher_tables(measurements, parent_ids)
    require(set(tables) == parent_ids, "ordinary-cp teacher table parent set mismatch")
    rows = []
    fallback_labels = []
    orders = (("candidate", "material"), ("material", "candidate"))
    for repetition, order in enumerate(orders):
        for parent_id in plan["parent_ids"]:
            table = tables[parent_id]
            position = table["position"]
            top_score = max(table["scores"].values())
            for arm in order:
                result = search_time(
                    args.engine,
                    position,
                    args.candidate if arm == "candidate" else None,
                    args.time_ms,
                )
                move = result["bestmove"]
                score = table["scores"].get(move)
                if score is None:
                    label = depth7_score(
                        args.engine,
                        args.baseline,
                        position,
                        move,
                        prereg["teacher_contract"]["timeout_seconds_per_search"],
                    )
                    score = label["score_cp"]
                    fallback_labels.append(
                        {"parent_id": parent_id, "arm": arm, "move": move, **label}
                    )
                regret = top_score - score if abs(score) <= NORMAL_SCORE_ABS_MAX_CP else None
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
    require({row["parent_id"] for row in rows} == parent_ids, "same-time parent set mismatch")
    same_time = {arm: same_time_metrics(rows, arm) for arm in ("candidate", "material")}
    same_time_conditions = {
        "all_candidate_bestmoves_stable": (
            same_time["candidate"]["stable_parents"] == len(parent_ids)
        ),
        "all_material_bestmoves_stable": (
            same_time["material"]["stable_parents"] == len(parent_ids)
        ),
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
        "schema": "sekirei.q21t-same-time-measurements.v1",
        "status": "complete",
        "diagnostic_only": True,
        "strength_claim": False,
        "plan": bind(args.output_dir / "same-time-plan.json"),
        "rows": rows,
        "fallback_depth7_labels": fallback_labels,
    }
    atomic_write(args.output_dir / "same-time-measurements.json", measurements_document)

    static_pass = all(static_conditions.values())
    same_time_pass = all(same_time_conditions.values())
    passed = static_pass and same_time_pass
    decision = {
        "schema": "sekirei.q21t-top-choice-pilot-decision.v1",
        "status": "pass" if passed else "fail",
        "diagnostic_only": True,
        "strength_claim": False,
        "experiment_complete": True,
        "candidate_adopted": False,
        "development_match_authorized": passed,
        "q20_authorized": False,
        "static": {
            **static,
            "candidate_mean_regret_reduction": reduction,
            "conditions": static_conditions,
            "pass": static_pass,
        },
        "same_time": {
            **same_time,
            "conditions": same_time_conditions,
            "pass": same_time_pass,
        },
        "next_action": (
            "run only the preregistered development match"
            if passed
            else "reject the Q21t candidate and redesign the evaluator objective; do not run Q20"
        ),
        "artifacts": {
            name: bind(path)
            for name, path in {
                "preregistration": args.preregistration,
                "same_time_plan": args.output_dir / "same-time-plan.json",
                "train_pairs": args.train_pairs,
                "validation_pairs": args.validation_pairs,
                "validation_measurements": args.validation_measurements,
                "baseline_static_audit": args.output_dir / "static-baseline.json",
                "q21p_static_audit": args.output_dir / "static-q21p.json",
                "candidate": args.candidate,
                "candidate_training": args.candidate.with_suffix(".ranking.json"),
                "candidate_static_audit": args.output_dir / "static-candidate.json",
                "same_time_measurements": args.output_dir / "same-time-measurements.json",
                "runner": Path(__file__).resolve(),
            }.items()
        },
    }
    atomic_write(args.output_dir / "decision.json", decision)
    report = [
        "# Q21t direct top-choice pilot",
        "",
        "This is a diagnostic screen on a fresh holdout, not a strength result.",
        "",
        f"- Status: `{decision['status']}`.",
        f"- Static direct top regret: {baseline_mean:.3f} -> "
        f"{static['candidate']['mean_direct_top_regret_cp']:.3f} cp "
        f"({reduction * 100:.2f}% reduction).",
        f"- Static top-1 matches: {static['baseline']['top1_matches']} -> "
        f"{static['candidate']['top1_matches']} / {static['candidate']['parents']}.",
        f"- Static major regrets: {static['baseline']['major_regrets_ge_300cp']} -> "
        f"{static['candidate']['major_regrets_ge_300cp']}.",
        f"- Same-time mean regret, candidate/material: "
        f"{same_time['candidate']['mean_direct_top_regret_cp']:.3f} / "
        f"{same_time['material']['mean_direct_top_regret_cp']:.3f} cp.",
        f"- Same-time top-1 matches, candidate/material: "
        f"{same_time['candidate']['top1_matches']} / {same_time['material']['top1_matches']}.",
        f"- Development match authorized: {passed}.",
        "- Q20 remains unauthorized.",
        "",
    ]
    (args.output_dir / "report.md").write_text("\n".join(report), encoding="utf-8")
    return decision


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preregistration", type=Path, required=True)
    parser.add_argument("--train-pairs", type=Path, required=True)
    parser.add_argument("--validation-pairs", type=Path, required=True)
    parser.add_argument("--validation-measurements", type=Path, required=True)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--ranking-auditor", type=Path, required=True)
    parser.add_argument("--trainer", type=Path, required=True)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--old-candidate", type=Path, required=True)
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
