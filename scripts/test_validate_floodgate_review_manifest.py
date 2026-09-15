#!/usr/bin/env python3
import importlib.util
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("validator", ROOT / "scripts/validate_floodgate_review_manifest.py")
validator = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(validator)


def document():
    pair = {
        "id": "g",
        "status": "verified",
        "csa": {"path": "g.csa", "bytes": 1, "sha256": "a" * 64},
        "analysis": {"path": "g.jsonl", "bytes": 1, "sha256": "b" * 64},
        "evidence_status": {"raw_pair": "verified", "semantic_replay": "unknown", "evaluator": "unknown", "strength": "not_permitted"},
    }
    return {
        "schema": "sekirei.floodgate-review-manifest.v1",
        "diagnostic_only": True,
        "pairs": [pair],
        "summary": {"paired": 1, "verified": 1, "invalid": 0},
        "evidence_summary": {"raw_pair_verified": 1, "raw_pair_invalid": 0, "strength_claim": "not_permitted"},
    }


def test_accepts_consistent_manifest():
    assert validator.validate(document()) == []


def test_rejects_inconsistent_summary_and_evidence():
    value = document()
    value["summary"]["verified"] = 0
    value["evidence_summary"]["raw_pair_verified"] = 0
    errors = validator.validate(value)
    assert "summary.verified" in errors
    assert "evidence_summary.raw_pair_verified" in errors


if __name__ == "__main__":
    test_accepts_consistent_manifest()
    test_rejects_inconsistent_summary_and_evidence()
    print("PASS")
