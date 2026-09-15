#!/usr/bin/env python3
import json
import tempfile
from pathlib import Path

from validate_floodgate_budget_ladder import validate


def artifact() -> dict:
    transition = {
        "from_nodes": 10, "to_nodes": 20, "comparable": False,
        "bestmove_changed": None, "pv_changed": None, "bound_changed": None,
        "score_delta_cp": None, "depth_delta": None,
    }
    return {
        "schema": "sekirei.floodgate-budget-ladder-analysis.v1",
        "diagnostic_only": True, "budgets_nodes": [10, 20],
        "positions": [{"game_id": "g", "ply": 0,
                       "observations": [{"nodes": 10}, {"nodes": 20}],
                       "transitions": [transition]}],
        "summary": {"positions": 1, "transitions": 0, "bestmove_changes": 0,
                    "pv_changes": 0, "bound_changes": 0,
                    "nonzero_score_deltas": 0, "depth_increases": 0},
        "claims": {"strength": "not_permitted", "causal_inference": "not_proven"},
    }


with tempfile.TemporaryDirectory() as directory:
    path = Path(directory) / "artifact.json"
    path.write_text(json.dumps(artifact()), encoding="utf-8")
    validate(path)
    broken = artifact()
    broken["positions"][0]["transitions"][0]["score_delta_cp"] = 1
    path.write_text(json.dumps(broken), encoding="utf-8")
    try:
        validate(path)
    except ValueError:
        pass
    else:
        raise AssertionError("non-comparable result was accepted")
print("PASS")
