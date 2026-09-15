#!/usr/bin/env python3
import importlib.util
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "validator", ROOT / "scripts/validate_floodgate_holdout_comparison.py"
)
validator = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(validator)


def valid_document():
    return {
        "schema": "sekirei.floodgate-holdout-comparison.v1",
        "diagnostic_only": True,
        "decision": "not_evaluable",
        "claims": {"strength": "not_permitted", "played_move_is_label": False},
        "baseline_corpus_sha256": "a" * 64,
        "candidate_corpus_sha256": "b" * 64,
        "source_corpus_sha256": None,
        "execution_differences": [],
        "allowed_execution_differences": [],
        "counts": {"total": 1, "comparable": 1, "not_comparable": 0},
        "rows": [{
            "key": "holdout:0",
            "status": "comparable",
            "not_comparable_reasons": [],
            "bestmove_changed": False,
            "pv_changed": False,
            "score_delta_candidate_minus_baseline_cp": 0,
            "depth_delta_candidate_minus_baseline": 0,
        }],
    }


def test_accepts_legacy_comparison_without_integrity_fields():
    assert validator.validate(valid_document()) == []


def test_accepts_a_verified_integrity_pair():
    document = valid_document()
    document["rows"][0].update({
        "baseline_search_integrity": {
            "status": "verified", "pv_legal": True, "pv_replay_preserves_input": True,
        },
        "candidate_search_integrity": {
            "status": "verified", "pv_legal": True, "pv_replay_preserves_input": True,
        },
    })
    assert validator.validate(document) == []


def test_rejects_one_sided_integrity_and_bad_counts():
    document = valid_document()
    document["rows"][0]["baseline_search_integrity"] = {"status": "unknown", "missing": ["pv_legal"]}
    document["counts"]["comparable"] = 0
    errors = validator.validate(document)
    assert "rows[0].search_integrity_pair" in errors
    assert "counts" in errors


def test_rejects_partial_corpus_provenance():
    document = valid_document()
    document["source_corpus_sha256"] = "baseline"
    assert "source_corpus_sha256" in validator.validate(document)


def test_rejects_malformed_execution_difference_list():
    document = valid_document()
    document["execution_differences"] = ["nodes", 1]
    assert "execution_differences" in validator.validate(document)


def test_rejects_non_sha256_corpus_provenance():
    document = valid_document()
    document["baseline_corpus_sha256"] = "baseline"
    assert "baseline_corpus_sha256" in validator.validate(document)


if __name__ == "__main__":
    test_accepts_legacy_comparison_without_integrity_fields()
    test_accepts_a_verified_integrity_pair()
    test_rejects_one_sided_integrity_and_bad_counts()
    print("PASS")
