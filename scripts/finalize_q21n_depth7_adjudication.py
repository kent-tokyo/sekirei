#!/usr/bin/env python3
"""Finalize Q21n after its preregistered depth-7 adjudication."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any


MATE_THRESHOLD_CP = 899_000


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def bind(path: Path) -> dict[str, str]:
    return {"path": str(path), "sha256": sha256(path)}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def exact(result: dict[str, Any]) -> bool:
    return (
        result.get("completion") == "search_completed"
        and result.get("completed_iteration_valid") == "true"
        and result.get("completed_bound") == "exact"
        and result.get("pv_legal") is True
        and result.get("history_matches_expected") == "true"
    )


def classify(regrets: list[int], incomplete_or_mate: bool) -> str:
    if any(regret >= 300 for regret in regrets):
        return "teacher_depth_insufficient"
    if incomplete_or_mate or any(regret >= 100 for regret in regrets):
        return "mixed_teacher_and_student_risk"
    return "student_reproduction_primary"


def finalize(args: argparse.Namespace) -> dict[str, Any]:
    prereg = json.loads(args.preregistration.read_text(encoding="utf-8"))
    measurements = json.loads(args.measurements.read_text(encoding="utf-8"))
    require(
        prereg.get("schema") == "sekirei.q21n-depth7-adjudication-preregistration.v1",
        "unexpected depth-7 preregistration",
    )
    require(
        measurements.get("schema") == "sekirei.q21n-depth7-adjudication-measurements.v1"
        and measurements.get("preregistration", {}).get("sha256") == sha256(args.preregistration),
        "measurements are not bound to the preregistration",
    )
    require(len(measurements.get("rows", [])) == len(prereg["selected"]), "measurement count mismatch")
    rows = []
    regrets: list[int] = []
    incomplete_or_mate = False
    for row in measurements["rows"]:
        selection, free, fixed = row["selection"], row["free"], row["fixed_depth3_top"]
        expected_moves = set(selection["depth3_top_moves"])
        require(set(fixed) == expected_moves, f"{selection['position_id']}: fixed move set mismatch")
        all_results = [free, *fixed.values()]
        complete = all(exact(result) for result in all_results)
        mate_like = any(
            isinstance(result.get("score_cp"), int) and abs(result["score_cp"]) >= MATE_THRESHOLD_CP
            for result in all_results
        )
        regret = None
        if complete and not mate_like:
            regret = free["score_cp"] - max(result["score_cp"] for result in fixed.values())
            regrets.append(regret)
        else:
            incomplete_or_mate = True
        rows.append({
            **selection,
            "complete_exact": complete,
            "mate_like": mate_like,
            "depth7_free_bestmove": free.get("bestmove"),
            "depth7_free_score_cp": free.get("score_cp"),
            "depth7_fixed_scores_cp": {move: result.get("score_cp") for move, result in fixed.items()},
            "depth7_regret_cp": regret,
        })
    classification = classify(regrets, incomplete_or_mate)
    next_actions = {
        "teacher_depth_insufficient": "replace depth-3 root labels with a stronger teacher-search contract before retraining",
        "mixed_teacher_and_student_risk": "keep teacher and student causes separate; do not start another broad training recipe",
        "student_reproduction_primary": "retain fixed-T labels and focus the next pilot on student representation or optimization",
    }
    return {
        "schema": "sekirei.q21n-final-decision.v1",
        "status": "complete",
        "diagnostic_only": True,
        "strength_claim": False,
        "classification": classification,
        "reference_boundary": "depth-7 fixed T is a deeper self-reference, not ground truth",
        "summary": {
            "selected_parents": len(rows),
            "ordinary_complete_parents": len(regrets),
            "major_regret_ge_300cp": sum(regret >= 300 for regret in regrets),
            "moderate_regret_ge_100cp": sum(regret >= 100 for regret in regrets),
            "maximum_regret_cp": max(regrets) if regrets else None,
            "incomplete_or_mate_parents": sum(not row["complete_exact"] or row["mate_like"] for row in rows),
        },
        "next_action": next_actions[classification],
        "development_match_authorized": False,
        "q20_authorized": False,
        "rows": rows,
        "artifacts": {
            "preregistration": bind(args.preregistration),
            "measurements": bind(args.measurements),
            "finalizer": bind(Path(__file__).resolve()),
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preregistration", type=Path, required=True)
    parser.add_argument("--measurements", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        document = finalize(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"status": document["status"], "classification": document["classification"], **document["summary"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
