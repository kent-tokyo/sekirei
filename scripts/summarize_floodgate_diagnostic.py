#!/usr/bin/env python3
"""Summarize fixed-budget Floodgate diagnostics without making strength claims."""
from __future__ import annotations

import argparse
import json
from pathlib import Path


SCHEMA = "sekirei.floodgate-core-diagnostic.v2"


def classify(document: dict) -> dict:
    if document.get("schema") != SCHEMA or not document.get("diagnostic_only"):
        raise ValueError("input is not a diagnostic-only core report")
    rows = []
    for item in document.get("results", []):
        unrestricted = item.get("unrestricted", {})
        actual = item.get("actual_root", {})
        comparison = item.get("comparison", {})
        cold_warm = item.get("unrestricted_tt_comparison", {})
        score_delta = comparison.get("score_delta_actual_root_minus_unrestricted_cp")
        tt_changed = cold_warm.get("bestmove_changed")
        # Node-limited iterative deepening normally aborts a later pass.  It
        # remains comparable when both arms retain an exact completed pass;
        # using only the final `aborted` flag incorrectly classified every
        # fixed-budget result as incomplete.
        def completed_exact(search: dict) -> bool:
            return (search.get("completed_bound") == "exact"
                    and search.get("completed_iteration_valid") in {True, "true"})

        if not completed_exact(unrestricted) or not completed_exact(actual):
            diagnostic_class = "incomplete_search"
        elif score_delta is not None and score_delta != 0:
            diagnostic_class = "root_score_gap_observed"
        elif tt_changed:
            diagnostic_class = "tt_move_change_observed"
        else:
            diagnostic_class = "no_difference_observed"
        rows.append({
            "index": item.get("index"),
            "game_id": item.get("source", {}).get("game_id"),
            "ply": item.get("source", {}).get("ply"),
            "played_move": item.get("observed_move_usi"),
            "unrestricted_bestmove": unrestricted.get("bestmove"),
            "unrestricted_pv": unrestricted.get("pv_usi", []),
            "root_candidates": unrestricted.get("root_candidates", []),
            "actual_root_pv": actual.get("pv_usi", []),
            "unrestricted_bound": unrestricted.get("bound", "unknown"),
            "unrestricted_completed_bound": unrestricted.get("completed_bound", "unknown"),
            "unrestricted_completed_iteration_valid": completed_exact(unrestricted),
            "actual_root_bound": actual.get("bound", "unknown"),
            "actual_root_completed_bound": actual.get("completed_bound", "unknown"),
            "actual_root_completed_iteration_valid": completed_exact(actual),
            "unrestricted_aborted": unrestricted.get("aborted", False),
            "actual_root_aborted": actual.get("aborted", False),
            "bestmove_matches_played": comparison.get("unrestricted_bestmove_matches_played"),
            "score_delta_actual_minus_unrestricted_cp": comparison.get(
                "score_delta_actual_root_minus_unrestricted_cp"
            ),
            "depth_delta_actual_minus_unrestricted": comparison.get(
                "depth_delta_actual_root_minus_unrestricted"
            ),
            "tt_bestmove_changed": cold_warm.get("bestmove_changed"),
            "tt_score_delta_warm_minus_cold_cp": cold_warm.get(
                "score_delta_warm_minus_cold_cp"
            ),
            "diagnostic_class": diagnostic_class,
        })
    return {
        "schema": "sekirei.floodgate-diagnostic-summary.v1",
        "diagnostic_only": True,
        "source_schema": SCHEMA,
        "execution": document.get("execution"),
        "nodes": document.get("nodes"),
        "warmup_nodes": document.get("warmup_nodes", 0),
        "rows": rows,
        "claims": {
            "strength": "not_permitted",
            "played_move_is_label": False,
            "interpretation": "root and TT effects are hypotheses, not causal proof",
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    summary = classify(json.loads(args.input.read_text(encoding="utf-8")))
    args.output.write_text(json.dumps(summary, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {len(summary['rows'])} diagnostic rows")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
