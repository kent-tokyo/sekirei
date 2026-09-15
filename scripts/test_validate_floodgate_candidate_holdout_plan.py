import importlib.util
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("validator", ROOT / "scripts/validate_floodgate_candidate_holdout_plan.py")
module = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(module)


def valid():
    return {
        "schema": "sekirei.floodgate-candidate-holdout-plan.v1", "status": "planned",
        "execution_performed": False, "candidate": {"version": "0.3.36"},
        "split": {"holdout_entries": 8},
        "protocol": {"Threads": "1", "SpecTopN": "0", "UseBook": "false", "SearchMode": "Speculative",
                     "preflight_required": True, "formal_gate": False},
        "claims": {"strength": "not_permitted", "candidate_adoption": "not_established"},
    }


def test_valid():
    assert module.validate(valid()) == []


def test_rejects_execution():
    document = valid()
    document["execution_performed"] = True
    assert "execution_performed" in module.validate(document)


if __name__ == "__main__":
    test_valid()
    test_rejects_execution()
    print("PASS")
