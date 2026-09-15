#!/usr/bin/env python3
import json
import tempfile
from pathlib import Path

from analyze_floodgate_budget_ladder import analyze


def report(nodes: int, score: int, pv: list[str], aborted: bool = False, bound: str = "exact") -> dict:
    return {
        "schema": "sekirei.floodgate-core-diagnostic.v2",
        "diagnostic_only": True,
        "nodes": nodes,
        "execution": {"options": {"threads": 1, "spec_top_n": 0, "use_book": False,
                                     "null_move_pruning": True, "max_depth": None}, "weights": None},
        "results": [{"source": {"game_id": "g", "ply": 0}, "completion": "search_completed",
                     "bestmove": "7g7f", "score_cp": score, "depth": 2,
                     "bound": bound, "aborted": aborted, "pv_usi": pv,
                     "nodes": nodes, "elapsed_ms": 1}],
    }


with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    paths = []
    for nodes, score, pv in [(20, 0, ["7g7f"]), (40, 30, ["7g7f", "3c3d"]), (80, 30, ["7g7f", "3c3d"])]:
        path = root / f"{nodes}.json"
        path.write_text(json.dumps(report(nodes, score, pv)), encoding="utf-8")
        paths.append(path)
    result = analyze(paths)
    assert result["summary"] == {
        "positions": 1,
        "transitions": 2,
        "bestmove_changes": 0,
        "pv_changes": 1,
        "bound_changes": 0,
        "nonzero_score_deltas": 1,
        "depth_increases": 0,
    }
print("PASS")

with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    paths = []
    for nodes in (20, 40):
        path = root / f"aborted-{nodes}.json"
        path.write_text(json.dumps(report(nodes, 0, ["7g7f"], aborted=True, bound="unknown")), encoding="utf-8")
        paths.append(path)
    result = analyze(paths)
    assert result["summary"]["transitions"] == 0
    assert result["positions"][0]["transitions"][0]["comparable"] is False
print("PASS aborted-bound-excluded")
