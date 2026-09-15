#!/usr/bin/env python3
"""Validate a budget-ladder diagnostic artifact without strength claims."""
from __future__ import annotations

import argparse
import json
from pathlib import Path


SCHEMA = "sekirei.floodgate-budget-ladder-analysis.v1"


def validate(path: Path) -> None:
    document = json.loads(path.read_text(encoding="utf-8"))
    if document.get("schema") != SCHEMA or document.get("diagnostic_only") is not True:
        raise ValueError("artifact must be diagnostic-only budget ladder schema")
    budgets = document.get("budgets_nodes")
    if not isinstance(budgets, list) or len(budgets) < 2 or budgets != sorted(set(budgets)):
        raise ValueError("budgets_nodes must be ascending and unique")
    positions = document.get("positions")
    if not isinstance(positions, list) or not positions:
        raise ValueError("positions must be non-empty")
    expected_transitions = len(budgets) - 1
    for position in positions:
        observations = position.get("observations")
        transitions = position.get("transitions")
        if not isinstance(observations, list) or len(observations) != len(budgets):
            raise ValueError("each position must have one observation per budget")
        if not isinstance(transitions, list) or len(transitions) != expected_transitions:
            raise ValueError("each position must have one transition per adjacent budget")
        for index, observation in enumerate(observations):
            actual_nodes = observation.get("nodes")
            if not isinstance(actual_nodes, int) or actual_nodes < 0:
                raise ValueError("observation nodes must be a non-negative integer")
        for index, transition in enumerate(transitions):
            if transition.get("from_nodes") != budgets[index] or transition.get("to_nodes") != budgets[index + 1]:
                raise ValueError("transition budget mismatch")
            comparable = transition.get("comparable")
            if not isinstance(comparable, bool):
                raise ValueError("transition comparable must be boolean")
            for field in ("bestmove_changed", "pv_changed", "bound_changed", "score_delta_cp", "depth_delta"):
                if not comparable and transition.get(field) is not None:
                    raise ValueError("non-comparable transition contains comparison result")
    summary = document.get("summary", {})
    all_transitions = [transition for position in positions for transition in position["transitions"]]
    comparable = [item for item in all_transitions if item["comparable"]]
    expected = {
        "positions": len(positions),
        "transitions": len(comparable),
        "bestmove_changes": sum(item["bestmove_changed"] is True for item in comparable),
        "pv_changes": sum(item["pv_changed"] is True for item in comparable),
        "bound_changes": sum(item["bound_changed"] is True for item in comparable),
        "nonzero_score_deltas": sum(item["score_delta_cp"] not in (None, 0) for item in comparable),
        "depth_increases": sum((item["depth_delta"] or 0) > 0 for item in comparable),
    }
    if summary != expected:
        raise ValueError(f"summary mismatch: expected {expected}, got {summary}")
    claims = document.get("claims", {})
    if claims.get("strength") != "not_permitted" or claims.get("causal_inference") != "not_proven":
        raise ValueError("strength and causal claims must remain prohibited")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("artifact", type=Path)
    args = parser.parse_args()
    validate(args.artifact)
    print("PASS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
