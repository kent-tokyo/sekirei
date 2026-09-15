#!/usr/bin/env python3
import importlib.util
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("regression", ROOT / "scripts/evaluate_floodgate_holdout_regression.py")
module = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(module)


def test_clean_scope_is_not_strength_claim():
    result = module.evaluate({
        "schema": "sekirei.floodgate-holdout-comparison.v1", "diagnostic_only": True,
        "counts": {"total": 2, "comparable": 2},
        "source_corpus_sha256": "corpus",
        "rows": [{"bestmove_changed": False, "score_delta_candidate_minus_baseline_cp": 0},
                 {"bestmove_changed": False, "score_delta_candidate_minus_baseline_cp": 0}],
    })
    assert result["status"] == "clean_for_this_diagnostic"
    assert result["claims"]["improvement"] == "not_established"


def test_reports_search_integrity_without_promoting_it_to_strength():
    result = module.evaluate({
        "schema": "sekirei.floodgate-holdout-comparison.v1", "diagnostic_only": True,
        "counts": {"total": 1, "comparable": 1}, "source_corpus_sha256": "corpus",
        "rows": [{
            "bestmove_changed": False, "score_delta_candidate_minus_baseline_cp": 0,
            "baseline_search_integrity": {"status": "verified"},
            "candidate_search_integrity": {"status": "unknown"},
        }],
    })
    assert result["status"] == "inconclusive"
    assert result["search_integrity"] == {"verified": 1, "failed": 0, "unknown": 1}
    assert result["claims"]["strength"] == "not_permitted"


def test_rejects_invalid_search_integrity_status():
    result = module.evaluate({
        "schema": "sekirei.floodgate-holdout-comparison.v1", "diagnostic_only": True,
        "counts": {"total": 1, "comparable": 1}, "source_corpus_sha256": "corpus",
        "rows": [{
            "bestmove_changed": False, "score_delta_candidate_minus_baseline_cp": 0,
            "baseline_search_integrity": {"status": "maybe"},
        }],
    })
    assert result["status"] == "inconclusive"
    assert "search_integrity_invalid" in result["reasons"]


def test_rejects_one_sided_search_integrity():
    result = module.evaluate({
        "schema": "sekirei.floodgate-holdout-comparison.v1", "diagnostic_only": True,
        "counts": {"total": 1, "comparable": 1}, "source_corpus_sha256": "corpus",
        "rows": [{
            "bestmove_changed": False, "score_delta_candidate_minus_baseline_cp": 0,
            "baseline_search_integrity": {"status": "verified"},
        }],
    })
    assert result["status"] == "inconclusive"
    assert "search_integrity_invalid" in result["reasons"]


if __name__ == "__main__":
    test_clean_scope_is_not_strength_claim()
    print("PASS")
