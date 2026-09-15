import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from validate_floodgate_hypothesis_selection import validate


valid = {
    "schema": "sekirei.floodgate-hypothesis-selection.v1",
    "diagnostic_only": True,
    "selected": {key: "value" for key in ("id", "rationale", "expected_change", "rejection_condition", "next_test")},
    "claims": {"strength": "not_permitted", "causal_inference": "not_proven", "implementation_adoption": "not_yet"},
}
assert validate(valid) == []
invalid = {**valid, "claims": {**valid["claims"], "implementation_adoption": "adopted"}}
assert "claims.implementation_adoption" in validate(invalid)
print("PASS")
