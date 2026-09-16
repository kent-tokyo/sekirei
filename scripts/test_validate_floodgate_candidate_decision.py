#!/usr/bin/env python3
import importlib.util
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "validator", ROOT / "scripts/validate_floodgate_candidate_decision.py"
)
validator = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(validator)


def conservative_decision():
    return {
        "schema": "sekirei.floodgate-candidate-decision.v1",
        "diagnostic_only": True,
        "status": "inconclusive",
        "decision": "keep_current",
        "candidate": {"version": "0.3.36", "formal_adoption": False},
        "evidence": {
            "pilot_manifest": "pilot.json", "holdout_manifest": "holdout.json",
            "gate_status": "ready_for_holdout", "holdout_regression": "clean",
            "nmp_ablation": "no_difference", "resource_preflight": "refuse",
            "version_alignment": "historical_evidence_not_candidate",
            "candidate_manifest": "candidate.json",
            "holdout_alignment_artifact": "alignment.json",
        },
        "reasons": ["test"],
        "next_action": "test",
        "claims": {
            "strength": "not_permitted",
            "release_approval": "not_granted",
            "competitor_superiority": "not_established",
        },
    }


def test_current_decision_is_conservative():
    assert validator.validate(conservative_decision()) == []


def test_strength_claim_cannot_be_reclassified():
    document = conservative_decision()
    document["claims"]["strength"] = "established"
    assert "strength claim" in validator.validate(document)


if __name__ == "__main__":
    test_current_decision_is_conservative()
    test_strength_claim_cannot_be_reclassified()
    print("PASS")
