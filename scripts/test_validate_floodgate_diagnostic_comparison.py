#!/usr/bin/env python3
import importlib.util


ROOT = __import__("pathlib").Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("validator", ROOT / "scripts/validate_floodgate_diagnostic_comparison.py")
module = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(module)


def valid():
    return {
        "schema": "sekirei.floodgate-diagnostic-candidate-comparison.v1",
        "diagnostic_only": True, "decision": "not_evaluable",
        "claims": {"strength": "not_permitted"},
        "baseline_corpus_sha256": "corpus", "candidate_corpus_sha256": "corpus",
        "source_corpus_sha256": "corpus",
        "counts": {"total": 1, "comparable": 1, "not_comparable": 0},
        "rows": [{
            "game_id": "g", "ply": 1, "status": "comparable",
            "not_comparable_reasons": [], "baseline_class": "no_difference_observed",
            "candidate_class": "no_difference_observed", "bestmove_changed": False,
            "baseline_root_candidate_moves": ["7g7f"], "candidate_root_candidate_moves": ["7g7f"],
            "root_candidate_set_changed": False, "root_candidate_scores_changed": False,
            "root_candidate_score_deltas_cp": {"7g7f": 0},
        }],
    }


def test_valid():
    assert module.validate(valid()) == []


def test_counts_and_duplicate_key_are_rejected():
    document = valid()
    document["counts"]["comparable"] = 0
    document["rows"].append(document["rows"][0].copy())
    assert "counts.comparable" in module.validate(document)
    assert "rows[1].duplicate_key" in module.validate(document)


if __name__ == "__main__":
    test_valid()
    test_counts_and_duplicate_key_are_rejected()
    print("PASS")
