import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from select_floodgate_hypothesis import select


base = {"schema": "sekirei.floodgate-budget-comparison.v1", "diagnostic_only": True,
        "summary": {"total": 4, "comparable": 4, "nonzero_score_changes": 2}}
chosen = select(base)
assert chosen["selected"]["id"] == "search_budget_sensitivity"
assert chosen["claims"]["implementation_adoption"] == "not_yet"
none = select({**base, "summary": {"total": 4, "comparable": 4, "nonzero_score_changes": 0}})
assert none["selected"]["id"] == "no_observed_budget_effect"
incomplete = select({**base, "summary": {"total": 4, "comparable": 0, "nonzero_score_changes": 0}})
assert incomplete["selected"]["id"] == "measurement_completion"
print("PASS")
