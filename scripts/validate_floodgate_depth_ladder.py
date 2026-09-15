#!/usr/bin/env python3
"""Validate a fixed-depth PV legality diagnostic artifact."""
from __future__ import annotations

import argparse
import json
from pathlib import Path


SCHEMA = "sekirei.floodgate-depth-ladder-analysis.v1"


def validate(path: Path) -> None:
    document = json.loads(path.read_text(encoding="utf-8"))
    if document.get("schema") != SCHEMA or document.get("diagnostic_only") is not True:
        raise ValueError("artifact must be diagnostic-only depth ladder schema")
    depths = document.get("depths")
    if not isinstance(depths, list) or len(depths) < 2 or depths != sorted(set(depths)):
        raise ValueError("depths must be ascending and unique")
    positions = document.get("positions")
    if not isinstance(positions, list) or not positions:
        raise ValueError("positions must be non-empty")
    transitions = []
    for position in positions:
        observations = position.get("observations")
        position_transitions = position.get("transitions")
        if not isinstance(observations, list) or len(observations) != len(depths):
            raise ValueError("each position needs one observation per depth")
        if not isinstance(position_transitions, list) or len(position_transitions) != len(depths) - 1:
            raise ValueError("each position needs one transition per adjacent depth")
        for index, observation in enumerate(observations):
            if observation.get("depth") != depths[index]:
                raise ValueError("observation depth mismatch")
            if observation.get("pv_legal") not in (True, "true"):
                raise ValueError("PV legality is not verified")
            if observation.get("pv_replay_preserves_input") not in (True, "true"):
                raise ValueError("PV replay mutated the input board")
        for index, transition in enumerate(position_transitions):
            if transition.get("from_depth") != depths[index] or transition.get("to_depth") != depths[index + 1]:
                raise ValueError("transition depth mismatch")
            comparable = transition.get("comparable")
            if not isinstance(comparable, bool):
                raise ValueError("transition comparable must be boolean")
            if not comparable:
                if any(transition.get(field) is not None for field in (
                    "bestmove_changed", "pv_changed", "bound_changed", "score_delta_cp", "nodes_delta")):
                    raise ValueError("non-comparable transition has comparison values")
            transitions.append(transition)
    comparable = [item for item in transitions if item["comparable"]]
    expected = {
        "positions": len(positions), "transitions": len(comparable),
        "bestmove_changes": sum(item["bestmove_changed"] is True for item in comparable),
        "pv_changes": sum(item["pv_changed"] is True for item in comparable),
        "bound_changes": sum(item["bound_changed"] is True for item in comparable),
        "nonzero_score_deltas": sum(item["score_delta_cp"] != 0 for item in comparable),
    }
    if document.get("summary") != expected:
        raise ValueError(f"summary mismatch: expected {expected}, got {document.get('summary')}")
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
