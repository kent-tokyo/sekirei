#!/usr/bin/env python3
import json
import tempfile
from pathlib import Path

from validate_floodgate_depth_ladder import validate


def make() -> dict:
    return {"schema": "sekirei.floodgate-depth-ladder-analysis.v1", "diagnostic_only": True,
        "depths": [2, 3], "positions": [{"observations": [
            {"depth": 2, "pv_legal": True, "pv_replay_preserves_input": True},
            {"depth": 3, "pv_legal": True, "pv_replay_preserves_input": True}],
            "transitions": [{"from_depth": 2, "to_depth": 3, "comparable": True,
              "bestmove_changed": False, "pv_changed": False, "bound_changed": False,
              "score_delta_cp": 0, "nodes_delta": 1}]}],
        "summary": {"positions": 1, "transitions": 1, "bestmove_changes": 0,
          "pv_changes": 0, "bound_changes": 0, "nonzero_score_deltas": 0},
        "claims": {"strength": "not_permitted", "causal_inference": "not_proven"}}


with tempfile.TemporaryDirectory() as directory:
    path = Path(directory) / "artifact.json"
    value = make()
    path.write_text(json.dumps(value), encoding="utf-8")
    validate(path)
    value["positions"][0]["observations"][0]["pv_legal"] = False
    path.write_text(json.dumps(value), encoding="utf-8")
    try:
        validate(path)
    except ValueError:
        pass
    else:
        raise AssertionError("illegal PV was accepted")
print("PASS")
