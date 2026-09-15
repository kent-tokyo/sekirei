#!/usr/bin/env python3
import json
import tempfile
from pathlib import Path

from analyze_floodgate_depth_ladder import analyze


def report(depth: int, score: int) -> dict:
    return {
        "schema": "sekirei.floodgate-core-diagnostic.v2", "diagnostic_only": True,
        "execution": {"weights": None, "options": {"max_depth": depth, "threads": 1,
          "spec_top_n": 0, "use_book": False, "null_move_pruning": True}},
        "results": [{"index": 0, "source": {"game_id": "g"}, "unrestricted": {
          "completion": "search_completed", "bestmove": "7g7f", "score_cp": score,
          "depth": depth, "bound": "exact", "aborted": False, "pv_usi": ["7g7f"],
          "pv_legal": True, "pv_replay_preserves_input": True,
          "nodes": depth * 10, "elapsed_ms": 1}}],
    }


with tempfile.TemporaryDirectory() as directory:
    root = Path(directory)
    paths = []
    for depth, score in ((2, 0), (3, 20), (4, 20)):
        path = root / f"{depth}.json"
        path.write_text(json.dumps(report(depth, score)), encoding="utf-8")
        paths.append(path)
    result = analyze(paths)
    assert result["summary"] == {"positions": 1, "transitions": 2,
        "bestmove_changes": 0, "pv_changes": 0, "bound_changes": 0,
        "nonzero_score_deltas": 1}
print("PASS")
