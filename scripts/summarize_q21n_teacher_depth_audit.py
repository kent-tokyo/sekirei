#!/usr/bin/env python3
"""Validate and summarize Q21n's complete-root depth-3/depth-5 audit."""

from __future__ import annotations

import argparse
import hashlib
import json
import statistics
from collections import defaultdict
from pathlib import Path
from typing import Any


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def bind(path: Path) -> dict[str, str]:
    return {"path": str(path), "sha256": sha256(path)}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def candidate_scores(row: dict[str, Any], score_limit: int) -> dict[str, int]:
    candidates = row.get("teacher_root", {}).get("root_candidates", [])
    require(isinstance(candidates, list) and candidates, f"{row.get('id')}: no root candidates")
    scores: dict[str, int] = {}
    for candidate in candidates:
        move, score = candidate.get("move"), candidate.get("score_cp")
        require(
            isinstance(move, str) and isinstance(score, int)
            and candidate.get("depth", 0) > 0
            and candidate.get("bound") == "exact"
            and candidate.get("abort_reason") == "none",
            f"{row.get('id')}: incomplete root candidate",
        )
        require(move not in scores, f"{row.get('id')}: duplicate root move {move}")
        scores[move] = score
    require(
        row.get("candidate_prefix_complete") is True
        and row.get("complete_legal_root_set") is True
        and row.get("teacher_root", {}).get("root_legal_move_count") == len(scores),
        f"{row.get('id')}: incomplete legal root set",
    )
    # Mate-like moves stay in the raw evidence but cannot enter cp regret/order metrics.
    return {move: score for move, score in scores.items() if abs(score) <= score_limit}


def top_set(scores: dict[str, int]) -> set[str]:
    best = max(scores.values())
    return {move for move, score in scores.items() if score == best}


def pairwise_agreement(left: dict[str, int], right: dict[str, int]) -> tuple[int, int]:
    moves = sorted(left)
    agreed = comparable = 0
    for index, first in enumerate(moves):
        for second in moves[index + 1:]:
            left_delta = left[first] - left[second]
            right_delta = right[first] - right[second]
            if left_delta == 0 or right_delta == 0:
                continue
            comparable += 1
            agreed += (left_delta > 0) == (right_delta > 0)
    return agreed, comparable


def top_k(scores: dict[str, int], k: int) -> set[str]:
    return {move for move, _ in sorted(scores.items(), key=lambda item: (-item[1], item[0]))[:k]}


def summarize_rows(rows: list[dict[str, Any]]) -> dict[str, Any]:
    regrets = [row["deep_regret_cp"] for row in rows if row["ordinary_cp_comparable"]]
    ranks = [row["deep_rank_of_best_shallow_top"] for row in rows if row["ordinary_cp_comparable"]]
    agreed = sum(row["pairwise_agreed"] for row in rows)
    comparable = sum(row["pairwise_comparable"] for row in rows)
    return {
        "positions": len(rows),
        "ordinary_cp_positions": len(regrets),
        "top_set_agreements": sum(row["top_sets_overlap"] for row in rows),
        "top_set_agreement_rate": (
            sum(row["top_sets_overlap"] for row in rows) / len(rows) if rows else None
        ),
        "major_regret_ge_300cp": sum(regret >= 300 for regret in regrets),
        "moderate_regret_ge_100cp": sum(regret >= 100 for regret in regrets),
        "median_deep_regret_cp": statistics.median(regrets) if regrets else None,
        "mean_deep_regret_cp": statistics.fmean(regrets) if regrets else None,
        "maximum_deep_regret_cp": max(regrets) if regrets else None,
        "median_deep_rank_of_best_shallow_top": statistics.median(ranks) if ranks else None,
        "pairwise_order_agreement": agreed / comparable if comparable else None,
        "pairwise_comparisons": comparable,
        "mean_top8_overlap": statistics.fmean(row["top8_overlap"] for row in rows) if rows else None,
    }


def audit(args: argparse.Namespace) -> dict[str, Any]:
    prereg = json.loads(args.preregistration.read_text(encoding="utf-8"))
    shallow = json.loads(args.shallow.read_text(encoding="utf-8"))
    deep = json.loads(args.deep.read_text(encoding="utf-8"))
    forcing = json.loads(args.forcing.read_text(encoding="utf-8"))
    exclusion = json.loads(args.timeout_exclusion.read_text(encoding="utf-8"))
    require(
        prereg.get("schema") == "sekirei.q21n-teacher-depth-preregistration.v1"
        and prereg.get("status") == "frozen_before_deep_labels",
        "unexpected Q21n preregistration",
    )
    for name, path in (("shallow", args.shallow), ("forcing", args.forcing)):
        require(prereg["inputs"][name]["sha256"] == sha256(path), f"{name} SHA differs from preregistration")
    contract = prereg["deep_contract"]
    require(
        exclusion.get("schema") == "sekirei.q21n-timeout-exclusion.v1"
        and exclusion.get("status") == "frozen_before_completed-score-inspection"
        and exclusion.get("completion_contract", {}).get("accepted_complete_parents") == 17
        and exclusion.get("completion_contract", {}).get("resource_censored_parents") == 1,
        "unexpected Q21n timeout exclusion",
    )
    require(
        exclusion["inputs"]["initial_deep"]["sha256"] == sha256(args.deep),
        "timeout exclusion is not bound to the deep artifact",
    )
    excluded_id = exclusion["completion_contract"]["excluded_position_id"]
    require(
        deep.get("schema") == "sekirei.root-rank-teacher-corpus.v1"
        and deep.get("contract", {}).get("depth") == contract["depth"]
        and deep.get("contract", {}).get("root_candidate_limit") == contract["root_candidate_limit"]
        and deep.get("contract", {}).get("complete_legal_root_set") is False,
        "deep artifact does not match the one-timeout completion contract",
    )
    for field in ("binary_sha256", "weights_sha256", "nnue_output"):
        require(
            deep.get("teacher", {}).get(field) == shallow.get("teacher", {}).get(field),
            f"deep and shallow teacher {field} differ",
        )
    require(
        deep.get("source_corpus", {}).get("sha256") == prereg["inputs"]["corpus"]["sha256"],
        "deep source corpus differs from preregistration",
    )
    shallow_rows = {row["id"]: row for row in shallow.get("rows", [])}
    deep_rows = {row["id"]: row for row in deep.get("rows", [])}
    forcing_rows = {row["id"]: row for row in forcing.get("entries", [])}
    require(
        set(shallow_rows) == set(deep_rows) == set(forcing_rows)
        and len(shallow_rows) == prereg["selection"]["positions"],
        "shallow, deep, and forcing parent IDs differ",
    )
    incomplete_ids = {
        identifier for identifier, row in deep_rows.items()
        if row.get("candidate_prefix_complete") is not True
        or row.get("complete_legal_root_set") is not True
    }
    require(incomplete_ids == {excluded_id}, "deep artifact has an unregistered incomplete parent")
    accepted_ids = set(deep_rows) - incomplete_ids
    require(len(accepted_ids) == 17, "Q21n requires 17 complete accepted parents")

    score_limit = prereg["metrics"]["ordinary_cp_limit"]
    rows = []
    for position_id in sorted(accepted_ids):
        shallow_row, deep_row = shallow_rows[position_id], deep_rows[position_id]
        shallow_scores = candidate_scores(shallow_row, score_limit)
        deep_scores = candidate_scores(deep_row, score_limit)
        raw_shallow_moves = {
            candidate["move"] for candidate in shallow_row["teacher_root"]["root_candidates"]
        }
        raw_deep_moves = {
            candidate["move"] for candidate in deep_row["teacher_root"]["root_candidates"]
        }
        require(raw_shallow_moves == raw_deep_moves, f"{position_id}: legal move set changed")
        ordinary = bool(shallow_scores) and set(shallow_scores) == set(deep_scores)
        agreed = comparable = 0
        regret = rank = None
        shallow_top: set[str] = set()
        deep_top: set[str] = set()
        overlap = 0
        if ordinary:
            shallow_top, deep_top = top_set(shallow_scores), top_set(deep_scores)
            chosen_deep_score = max(deep_scores[move] for move in shallow_top)
            regret = max(deep_scores.values()) - chosen_deep_score
            rank = 1 + sum(score > chosen_deep_score for score in deep_scores.values())
            agreed, comparable = pairwise_agreement(shallow_scores, deep_scores)
            overlap = len(top_k(shallow_scores, 8) & top_k(deep_scores, 8))
        category = shallow_row.get("category", "unknown/unknown")
        phase, separator, material = category.partition("/")
        require(bool(separator), f"{position_id}: invalid phase/material category")
        rows.append({
            "position_id": position_id,
            "category": category,
            "phase": phase,
            "material_band": material,
            "forcing_class": forcing_rows[position_id]["forcing_class"],
            "legal_moves": len(raw_shallow_moves),
            "ordinary_cp_comparable": ordinary,
            "shallow_top_moves": sorted(shallow_top),
            "deep_top_moves": sorted(deep_top),
            "top_sets_overlap": bool(shallow_top & deep_top) if ordinary else False,
            "deep_regret_cp": regret,
            "deep_rank_of_best_shallow_top": rank,
            "pairwise_agreed": agreed,
            "pairwise_comparable": comparable,
            "pairwise_order_agreement": agreed / comparable if comparable else None,
            "top8_overlap": overlap,
        })

    overall = summarize_rows(rows)
    minimum = prereg["decision_rule"]["minimum_ordinary_positions"]
    require(overall["ordinary_cp_positions"] >= minimum, "too few ordinary positions for Q21n decision")
    ordinary_count = overall["ordinary_cp_positions"]
    major_rate = overall["major_regret_ge_300cp"] / ordinary_count
    if major_rate >= 0.25:
        classification = "teacher_depth_insufficient"
        next_action = "increase or redesign teacher search before another student-training pilot"
    elif overall["moderate_regret_ge_100cp"] > 0:
        classification = "mixed_teacher_and_student_risk"
        next_action = "adjudicate the moderate-regret parents before changing the student recipe"
    else:
        classification = "student_reproduction_primary"
        next_action = "keep fixed-T labels and diagnose student features, optimization, and quantization"

    stratified: dict[str, dict[str, Any]] = {}
    for field in ("phase", "material_band", "forcing_class"):
        buckets: defaultdict[str, list[dict[str, Any]]] = defaultdict(list)
        for row in rows:
            buckets[row[field]].append(row)
        stratified[field] = {key: summarize_rows(value) for key, value in sorted(buckets.items())}
    return {
        "schema": "sekirei.q21n-teacher-depth-audit.v1",
        "status": "complete",
        "diagnostic_only": True,
        "strength_claim": False,
        "reference_boundary": "depth-5 fixed T is a deeper self-reference, not ground truth",
        "coverage": {
            "frozen_parents": 18,
            "complete_parents": 17,
            "resource_censored_parents": 1,
            "resource_censored_position_id": excluded_id,
            "resource_censored_reason": exclusion["reason"],
        },
        "overall": {**overall, "major_regret_rate": major_rate},
        "classification": classification,
        "next_action": next_action,
        "development_match_authorized": False,
        "q20_authorized": False,
        "stratified": stratified,
        "rows": rows,
        "artifacts": {
            "preregistration": bind(args.preregistration),
            "shallow": bind(args.shallow),
            "deep": bind(args.deep),
            "forcing": bind(args.forcing),
            "timeout_exclusion": bind(args.timeout_exclusion),
            "summarizer": bind(Path(__file__).resolve()),
        },
    }


def report(document: dict[str, Any]) -> str:
    overall = document["overall"]
    lines = [
        "# Q21n teacher-depth consistency audit",
        "",
        "This is a fixed-teacher diagnostic, not a strength result.",
        "",
        "## Result",
        "",
        f"- Classification: `{document['classification']}`.",
        f"- Complete parents: {overall['positions']}/18; one 479-legal-move parent was resource-censored after two label-free timeouts.",
        f"- Ordinary-cp comparable: {overall['ordinary_cp_positions']}.",
        f"- Depth-3/depth-5 top-set agreement: {overall['top_set_agreements']}/{overall['positions']} "
        f"({overall['top_set_agreement_rate']:.1%}).",
        f"- Deep regret >=300cp: {overall['major_regret_ge_300cp']}/{overall['ordinary_cp_positions']} "
        f"({overall['major_regret_rate']:.1%}); >=100cp: {overall['moderate_regret_ge_100cp']}.",
        f"- Median/maximum deep regret: {overall['median_deep_regret_cp']}/{overall['maximum_deep_regret_cp']} cp.",
        f"- Pairwise order agreement: {overall['pairwise_order_agreement']:.1%} "
        f"across {overall['pairwise_comparisons']:,} non-tied comparisons.",
        f"- Next: {document['next_action']}.",
        "",
        "Depth 5 is a deeper fixed-T self-reference, not a correct-move oracle. Development match and Q20 remain unauthorized.",
    ]
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preregistration", type=Path, required=True)
    parser.add_argument("--shallow", type=Path, required=True)
    parser.add_argument("--deep", type=Path, required=True)
    parser.add_argument("--forcing", type=Path, required=True)
    parser.add_argument("--timeout-exclusion", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    try:
        document = audit(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.write_text(
        json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    args.report.write_text(report(document), encoding="utf-8")
    print(json.dumps({
        "status": document["status"],
        "classification": document["classification"],
        "major_regret_rate": document["overall"]["major_regret_rate"],
    }, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
