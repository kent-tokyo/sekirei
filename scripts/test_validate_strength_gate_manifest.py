import importlib.util
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("gate_validator", ROOT / "scripts/validate_strength_gate_manifest.py")
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


def ranking_diagnostic():
    return {
        "schema": "sekirei.core-evaluator-comparison.v3",
        "diagnostic_only": True,
        "claims": {"strength": "not_permitted"},
        "inputs": {"baseline": "teacher.json", "candidate": "candidate.json"},
        "summary": {"total": 96, "comparable": 95},
    }


def test_accepts_diagnostic_only_ranking_calibration():
    assert MODULE.validate_calibration(ranking_diagnostic()) == []


def test_rejects_ranking_calibration_that_claims_strength():
    data = ranking_diagnostic()
    data["claims"]["strength"] = "claimed"
    assert MODULE.validate_calibration(data)
