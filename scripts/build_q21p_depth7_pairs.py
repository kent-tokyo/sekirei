#!/usr/bin/env python3
"""Validate Q21p A/A labels and derive strict adjacent ranking pairs."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


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
        and isinstance(result.get("score_cp"), int)
    )


def ordinary(result: dict[str, Any], score_limit: int) -> bool:
    return exact(result) and abs(result["score_cp"]) <= score_limit


def signature(result: dict[str, Any]) -> tuple[Any, ...]:
    return (
        result.get("bestmove"),
        result.get("score_cp"),
        result.get("depth"),
        result.get("completed_bound"),
    )


def build(prereg: dict[str, Any], measurements: dict[str, Any], measurements_path: Path) -> tuple[dict[str, Any], dict[str, Any]]:
    expected_status = {
        "sekirei.q21p-depth7-ranking-pilot-preregistration.v2": "frozen_before_depth7_labels",
        "sekirei.q21p-depth7-ranking-pilot-preregistration.v3": (
            "frozen_before_depth7_validation_labels_and_candidate_training"
        ),
        "sekirei.q21p-depth7-ranking-pilot-preregistration.v4": (
            "frozen_before_depth7_validation_labels_and_candidate_training"
        ),
    }.get(prereg.get("schema"))
    require(expected_status is not None and prereg.get("status") == expected_status, "unexpected Q21p preregistration")
    require(
        measurements.get("schema") == "sekirei.q21p-depth7-label-measurements.v1",
        "unexpected Q21p measurements",
    )
    rows = measurements.get("rows", [])
    require(len(rows) == prereg["parents"], "Q21p requires all preregistered parents")
    contract = prereg["candidate_contract"]
    pairs: list[dict[str, Any]] = []
    audit_rows = []
    seen_ids: set[str] = set()
    for row in rows:
        identifier = row.get("id")
        require(isinstance(identifier, str) and identifier not in seen_ids, "duplicate or missing parent id")
        seen_ids.add(identifier)
        free = row.get("free", [])
        fixed = row.get("fixed", [])
        require(
            len(free) == contract["free_repeats"]
            and len(fixed) == contract["fixed_root_repeats"],
            f"{identifier}: repeat count mismatch",
        )
        require(all(exact(result) for result in free), f"{identifier}: incomplete free search")
        require(
            all(signature(result) == signature(free[0]) for result in free[1:]),
            f"{identifier}: free-search A/A mismatch",
        )
        candidate_moves = row.get("candidate_moves", [])
        expected_union = set(row.get("shallow_top8_moves", [])) | {result["bestmove"] for result in free}
        require(set(candidate_moves) == expected_union, f"{identifier}: candidate union mismatch")
        require(
            all(set(repeat) == set(candidate_moves) for repeat in fixed),
            f"{identifier}: fixed-root candidate set mismatch",
        )
        for move in candidate_moves:
            results = [repeat[move] for repeat in fixed]
            require(all(exact(result) for result in results), f"{identifier}/{move}: incomplete fixed search")
            require(
                all(signature(result) == signature(results[0]) for result in results[1:]),
                f"{identifier}/{move}: fixed-search A/A mismatch",
            )
        scores = {
            move: fixed[0][move]["score_cp"]
            for move in candidate_moves
            if ordinary(fixed[0][move], contract["normal_score_abs_max_cp"])
        }
        if len(scores) < 2 and prereg["schema"].endswith(".v4"):
            audit_rows.append({
                "parent_id": identifier,
                "category": row["category"],
                "candidate_moves": len(candidate_moves),
                "ordinary_candidate_moves": len(scores),
                "mate_like_candidates_excluded": len(candidate_moves) - len(scores),
                "ranked_moves": 0,
                "strict_adjacent_pairs": 0,
                "free_bestmove": free[0]["bestmove"],
                "free_score_cp": free[0]["score_cp"],
                "free_score_is_ordinary_cp": abs(free[0]["score_cp"])
                <= contract["normal_score_abs_max_cp"],
                "aa_deterministic": True,
                "excluded_from_normal_cp_screen": True,
                "exclusion_reason": "fewer_than_two_ordinary_depth7_scores",
            })
            continue
        require(len(scores) >= 2, f"{identifier}: fewer than two ordinary fixed-root scores")
        ranked = sorted(scores.items(), key=lambda item: (-item[1], item[0]))[
            : contract["top_k_after_depth7_reranking"]
        ]
        parent_pairs = 0
        for (higher_move, higher_score), (lower_move, lower_score) in zip(ranked, ranked[1:]):
            if higher_score <= lower_score:
                continue
            pairs.append({
                "parent_id": identifier,
                "category": row["category"],
                "initial_sfen": row["initial_sfen"],
                "history_before_usi": row["history_before_usi"],
                "parent_sfen": row["sfen"],
                "source": row.get("source"),
                "higher_move_usi": higher_move,
                "lower_move_usi": lower_move,
                "teacher_score_gap_cp": higher_score - lower_score,
            })
            parent_pairs += 1
        require(parent_pairs > 0, f"{identifier}: no strict adjacent depth-7 pair")
        audit_rows.append({
            "parent_id": identifier,
            "category": row["category"],
            "candidate_moves": len(candidate_moves),
            "ordinary_candidate_moves": len(scores),
            "mate_like_candidates_excluded": len(candidate_moves) - len(scores),
            "ranked_moves": len(ranked),
            "strict_adjacent_pairs": parent_pairs,
            "free_bestmove": free[0]["bestmove"],
            "free_score_cp": free[0]["score_cp"],
            "free_score_is_ordinary_cp": abs(free[0]["score_cp"])
            <= contract["normal_score_abs_max_cp"],
            "aa_deterministic": True,
        })
    measurements_sha = sha256(measurements_path)
    teacher = prereg["teacher_contract"]
    output = {
        "schema": "sekirei.root-rank-pairs.v1",
        "diagnostic_only": True,
        "strength_claim": "not_permitted",
        "source_contract": {
            "depth": teacher["max_depth"],
            "threads": teacher["threads"],
            "spec_top_n": teacher["spec_top_n"],
            "root_candidate_mode": "preregistered_candidate_union",
            "root_candidate_limit": contract["maximum_moves_per_parent"],
            "complete_legal_root_set": False,
            "candidate_source_sha256": measurements_sha,
            "per_category_unique_positions": 2,
            "normal_score_abs_max_cp": contract["normal_score_abs_max_cp"],
        },
        "source_teacher": {
            "binary": prereg["inputs"]["engine"]["path"],
            "binary_sha256": teacher["binary_sha256"],
            "weights": prereg["inputs"]["weights"]["path"],
            "weights_sha256": teacher["weights_sha256"],
            "nnue_output": teacher["nnue_output"],
        },
        "pair_selection": contract["pair_selection"],
        "pairs": pairs,
    }
    preregistered_tool_sha = prereg["tools"]["pair_builder"]["sha256"]
    executed_tool_sha = sha256(Path(__file__).resolve())
    parents_with_pairs = sum(row["strict_adjacent_pairs"] > 0 for row in audit_rows)
    parents_excluded = len(audit_rows) - parents_with_pairs
    require(parents_with_pairs > 0, "no parent has a normal-cp ranking pair")
    audit = {
        "schema": "sekirei.q21p-depth7-label-audit.v1",
        "status": "pass",
        "diagnostic_only": True,
        "strength_claim": False,
        "parents": len(rows),
        "parents_with_pairs": parents_with_pairs,
        "parents_excluded_from_normal_cp_screen": parents_excluded,
        "pairs": len(pairs),
        "all_aa_deterministic": True,
        "all_free_bestmoves_in_candidate_union": True,
        "measurements": {"path": str(measurements_path), "sha256": measurements_sha},
        "tool_provenance": {
            "preregistered_sha256": preregistered_tool_sha,
            "executed_sha256": executed_tool_sha,
            "contract_preserving_fix_after_measurement": preregistered_tool_sha != executed_tool_sha,
            "fix_scope": (
                "exclude deterministic mate-like fixed-root candidates, and in v4 parents with "
                "fewer than two ordinary scores, from the ordinary-cp screen; the frozen "
                "normal_score_abs_max_cp contract is unchanged"
            ),
        },
        "rows": audit_rows,
    }
    return output, audit


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preregistration", type=Path, required=True)
    parser.add_argument("--measurements", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--audit-output", type=Path, required=True)
    args = parser.parse_args()
    try:
        prereg = json.loads(args.preregistration.read_text(encoding="utf-8"))
        measurements = json.loads(args.measurements.read_text(encoding="utf-8"))
        require(
            measurements.get("preregistration", {}).get("sha256") == sha256(args.preregistration),
            "measurements are not bound to the Q21p preregistration",
        )
        output, audit = build(prereg, measurements, args.measurements)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.write_text(json.dumps(output, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    args.audit_output.write_text(json.dumps(audit, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"status": audit["status"], "parents": audit["parents"], "pairs": audit["pairs"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
