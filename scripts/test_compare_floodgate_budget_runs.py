#!/usr/bin/env python3
import json
import tempfile
from pathlib import Path

from compare_floodgate_budget_runs import compare


def report(nodes, completion="search_completed", bestmove="7g7f", score=100, depth=2):
    return {"schema": "sekirei.floodgate-core-diagnostic.v1", "diagnostic_only": True,
            "nodes": nodes, "results": [{"source": {"game_id": "g", "ply": 3},
            "completion": completion, "bestmove": bestmove, "score_cp": score, "depth": depth}]}


with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    (root / "a.json").write_text(json.dumps(report(20_000)), encoding="utf-8")
    (root / "b.json").write_text(json.dumps(report(100_000, bestmove="2g2f", score=130, depth=3)), encoding="utf-8")
    result = compare(root / "a.json", root / "b.json")
    assert result["summary"] == {"total": 1, "comparable": 1, "incomplete": 0,
                                 "bestmove_changes": 1, "nonzero_score_changes": 1}
    (root / "c.json").write_text(json.dumps(report(100_000, completion="budget_before_depth_completion")), encoding="utf-8")
    incomplete = compare(root / "a.json", root / "c.json")
    assert incomplete["summary"]["comparable"] == 0
    assert incomplete["rows"][0]["bestmove_changed"] is None
print("PASS")
