#!/usr/bin/env python3
import importlib.util
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("gate", ROOT / "scripts/evaluate_floodgate_diagnostic_gate.py")
module = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(module)


def test_current_style_evidence_is_not_ready():
    result = module.evaluate(
        {"schema": "sekirei.floodgate-diagnostic-candidate-comparison.v1", "diagnostic_only": True,
         "decision": "not_evaluable", "counts": {"comparable": 0}},
        {"schema": "sekirei.floodgate-diagnostic-corpus-split.v1", "holdout_entry_indices": [1]},
    )
    assert result["status"] == "not_ready"
    assert "no_comparable_rows" in result["reasons"]
    assert result["claims"]["release_approval"] == "not_granted"


def test_ready_requires_comparable_rows_and_two_holdout_rows():
    result = module.evaluate(
        {"schema": "sekirei.floodgate-diagnostic-candidate-comparison.v1", "diagnostic_only": True,
         "decision": "not_evaluable", "counts": {"comparable": 2}},
        {"schema": "sekirei.floodgate-diagnostic-corpus-split.v1", "holdout_entry_indices": [2, 3]},
    )
    assert result["status"] == "ready_for_holdout"


def test_corpus_hash_mismatch_is_not_ready():
    result = module.evaluate(
        {"schema": "sekirei.floodgate-holdout-comparison.v1", "diagnostic_only": True,
         "decision": "not_evaluable", "counts": {"comparable": 2},
         "source_corpus_sha256": "before"},
        {"schema": "sekirei.floodgate-diagnostic-corpus-split.v1",
         "source_corpus_sha256": "after", "holdout_entry_indices": [2, 3]},
    )
    assert result["status"] == "not_ready"
    assert "corpus_hash_mismatch" in result["reasons"]


if __name__ == "__main__":
    test_current_style_evidence_is_not_ready()
    test_ready_requires_comparable_rows_and_two_holdout_rows()
    test_corpus_hash_mismatch_is_not_ready()
    print("PASS")
