import importlib.util
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location(
    "validator", ROOT / "scripts/validate_floodgate_hypothesis_consistency.py"
)
module = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(module)


def comparison():
    return {
        "schema": "sekirei.floodgate-budget-comparison.v1",
        "diagnostic_only": True,
        "summary": {"total": 4, "comparable": 4, "nonzero_score_changes": 2},
    }


def selection():
    return {
        "schema": "sekirei.floodgate-hypothesis-selection.v1",
        "diagnostic_only": True,
        "source_schema": "sekirei.floodgate-budget-comparison.v1",
        "source_summary": {"total": 4, "comparable": 4, "nonzero_score_changes": 2},
        "selected": {
            "id": "search_budget_sensitivity",
            "rationale": "score差が観測された",
            "expected_change": "差が減る",
            "rejection_condition": "差が再現しない",
            "next_test": "固定depth比較",
        },
        "claims": {
            "strength": "not_permitted",
            "causal_inference": "not_proven",
            "implementation_adoption": "not_yet",
        },
    }


def test_consistent():
    assert module.validate(comparison(), selection()) == []


def test_rejects_stale_summary():
    document = selection()
    document["source_summary"]["nonzero_score_changes"] = 0
    assert "source_summary.nonzero_score_changes" in module.validate(comparison(), document)


if __name__ == "__main__":
    test_consistent()
    test_rejects_stale_summary()
    print("PASS")
