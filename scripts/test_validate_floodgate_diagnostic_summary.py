#!/usr/bin/env python3
import importlib.util
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("validator", ROOT / "scripts/validate_floodgate_diagnostic_summary.py")
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


def valid():
    return {
        "schema": "sekirei.floodgate-diagnostic-summary.v1",
        "diagnostic_only": True,
        "claims": {"strength": "not_permitted"},
        "execution": {
            "source_revision": "abc", "binary": {"sha256": "bin"},
            "weights": None, "options": {"threads": 1}, "corpus_sha256": "corpus",
        },
        "nodes": 100, "warmup_nodes": 100,
        "rows": [{
            "diagnostic_class": "no_difference_observed",
            "unrestricted_aborted": False,
            "actual_root_aborted": False,
            "unrestricted_pv": [],
            "actual_root_pv": [],
        }],
    }


def test_valid_summary():
    assert MODULE.validate(valid()) == []


def test_root_candidates_are_validated_when_present():
    document = valid()
    document["rows"][0]["root_candidates"] = [{
        "move": "7g7f", "score_cp": 10, "depth": 2,
        "bound": "exact", "abort_reason": "none",
    }]
    assert MODULE.validate(document) == []
    document["rows"][0]["root_candidates"].append(
        document["rows"][0]["root_candidates"][0].copy()
    )
    assert any("root_candidates[1].move" in error for error in MODULE.validate(document))


def test_rejects_strength_claim_and_unknown_class():
    document = valid()
    document["claims"]["strength"] = "measured"
    document["rows"][0]["diagnostic_class"] = "strength_gain"
    errors = MODULE.validate(document)
    assert "claims.strength" in errors
    assert "rows[0].diagnostic_class" in errors


def test_rejects_missing_execution_contract():
    document = valid()
    del document["execution"]
    assert "execution" in MODULE.validate(document)


if __name__ == "__main__":
    test_valid_summary()
    test_rejects_strength_claim_and_unknown_class()
    print("PASS")
