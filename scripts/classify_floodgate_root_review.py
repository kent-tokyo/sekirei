#!/usr/bin/env python3
"""Classify root-review observations without making a causal claim."""
from __future__ import annotations

import argparse
import json
from collections import Counter
from pathlib import Path


NEXT_TEST = {
    "played_move_root_difference_observed": "same-position alternative-root comparison with a fixed child budget",
    "tt_bestmove_change_observed": "repeat cold/warm TT comparison in independent processes",
    "tt_score_change_observed": "repeat cold/warm score comparison with identical depth and node budget",
    "no_difference_observed": "add an independent control position before changing the engine",
    "incomplete_search": "raise the bounded node budget only after recording the abort boundary",
    "history_replay_unverified": "repair SFEN/history replay before any search comparison",
}


def classify_row(row: dict) -> str:
    comparison = row.get("comparison", {})
    searches = (row.get("cold"), row.get("warm"), row.get("actual_root"))
    if any(not isinstance(search, dict) or search.get("completion") != "search_completed" for search in searches):
        return "incomplete_search"
    if row.get("history_replay", {}).get("status") != "verified":
        return "history_replay_unverified"
    if comparison.get("warm_bestmove_changed"):
        return "tt_bestmove_change_observed"
    if comparison.get("warm_score_delta_cp", 0) != 0:
        return "tt_score_change_observed"
    if comparison.get("bestmove_matches_played") is False or comparison.get("actual_score_delta_cp", 0) != 0:
        return "played_move_root_difference_observed"
    return "no_difference_observed"


def classify(document: dict) -> dict:
    rows = document.get("rows", [])
    classifications = [classify_row(row) for row in rows]
    return {
        "schema": "sekirei.floodgate-root-review-classification.v1",
        "diagnostic_only": True,
        "source_schema": document.get("schema"),
        "rows": len(rows),
        "classification_counts": dict(sorted(Counter(classifications).items())),
        "classifications": [
            {"game_id": row.get("game_id"), "ply": row.get("ply"), "classification": category,
             "next_test": NEXT_TEST[category]}
            for row, category in zip(rows, classifications)
        ],
        "claims": {
            "causality": "not_established",
            "strength": "not_permitted",
            "played_move_label": "not_permitted",
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    document = json.loads(args.input.read_text(encoding="utf-8"))
    result = classify(document)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {result['rows']} rows")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
