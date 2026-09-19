#!/usr/bin/env python3
"""Derive strict pairwise ranking labels from a complete-root teacher corpus.

Rows without a verified complete legal-root set are never promoted into a pair.
The source scores are already measured from the parent's side to move; child
evaluation conversion is deliberately deferred to the trainer boundary.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path


SCHEMA = "sekirei.root-rank-pairs.v1"
PAIR_SELECTIONS = ("all", "adjacent")


def pairs(document: dict, selection: str = "all", top_k: int = 8) -> list[dict]:
    if selection not in PAIR_SELECTIONS:
        raise ValueError(f"unsupported pair selection: {selection}")
    if top_k < 2:
        raise ValueError("top_k must be at least two")
    if document.get("schema") != "sekirei.root-rank-teacher-corpus.v1":
        raise ValueError("unsupported root teacher corpus schema")
    if document.get("diagnostic_only") is not True:
        raise ValueError("root teacher corpus must be diagnostic_only")
    contract = document.get("contract")
    score_limit = contract.get("normal_score_abs_max_cp") if isinstance(contract, dict) else None
    if not isinstance(score_limit, int) or score_limit <= 0:
        raise ValueError("root teacher corpus lacks a positive normal_score_abs_max_cp")
    result: list[dict] = []
    for row in document.get("rows", []):
        if row.get("candidate_prefix_complete") is not True:
            continue
        if row.get("complete_legal_root_set") is not True:
            continue
        candidates = row.get("teacher_root", {}).get("root_candidates")
        if not isinstance(candidates, list):
            raise ValueError("completed row lacks root candidates")
        normalized = []
        for candidate in candidates:
            if not isinstance(candidate, dict):
                raise ValueError("invalid root candidate")
            move, score = candidate.get("move"), candidate.get("score_cp")
            if not isinstance(move, str) or not move or not isinstance(score, int):
                raise ValueError("root candidate lacks move/score")
            if abs(score) > score_limit:
                continue
            normalized.append((move, score))
        if len(normalized) < 2:
            continue
        ranked = sorted(normalized, key=lambda candidate: candidate[1], reverse=True)[:top_k]
        if selection == "all":
            selected = (
                (high_move, high_score, low_move, low_score)
                for high_move, high_score in ranked
                for low_move, low_score in ranked
                if high_score > low_score
            )
        else:
            selected = (
                (high_move, high_score, low_move, low_score)
                for (high_move, high_score), (low_move, low_score) in zip(ranked, ranked[1:])
                if high_score > low_score
            )
        for high_move, high_score, low_move, low_score in selected:
            result.append({
                "parent_id": row.get("id"),
                "category": row.get("category"),
                "initial_sfen": row.get("initial_sfen"),
                "history_before_usi": row.get("history_before_usi"),
                "parent_sfen": row.get("sfen"),
                # Preserve the replay provenance.  The split stage uses this
                # to keep all prefixes from one source game in one arm.
                "source": row.get("source"),
                "higher_move_usi": high_move,
                "lower_move_usi": low_move,
                "teacher_score_gap_cp": high_score - low_score,
            })
    if not result:
        raise ValueError("no strict root ranking pairs")
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--selection", choices=PAIR_SELECTIONS, default="all")
    parser.add_argument("--top-k", type=int, default=8, help="maximum teacher-ranked legal moves per parent")
    args = parser.parse_args()
    try:
        source = json.loads(args.input.read_text(encoding="utf-8"))
        output = {
            "schema": SCHEMA,
            "diagnostic_only": True,
            "strength_claim": "not_permitted",
            "source_contract": source.get("contract"),
            "source_teacher": source.get("teacher"),
            "pair_selection": args.selection,
            "pairs": pairs(source, args.selection, args.top_k),
        }
    except (OSError, ValueError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(output, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {len(output['pairs'])} strict pairs")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
