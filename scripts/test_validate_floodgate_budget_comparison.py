import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from validate_floodgate_budget_comparison import validate


valid = {
    "schema": "sekirei.floodgate-budget-comparison.v1", "diagnostic_only": True,
    "baseline": {"nodes": 20_000}, "candidate": {"nodes": 100_000},
    "rows": [{"game_id": "g", "ply": 3, "comparable": True,
              "bestmove_changed": False, "score_delta_candidate_minus_baseline_cp": 0,
              "depth_delta_candidate_minus_baseline": 1}],
    "summary": {"total": 1, "comparable": 1, "incomplete": 0,
                 "bestmove_changes": 0, "nonzero_score_changes": 0},
    "claims": {"strength": "not_permitted"},
}
assert validate(valid) == []
invalid = {**valid, "rows": [{**valid["rows"][0], "comparable": False}],
           "summary": {**valid["summary"], "comparable": 0, "incomplete": 1}}
assert "rows[0].incomplete_bestmove" in validate(invalid)
print("PASS")
