#!/usr/bin/env python3
import importlib.util
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("comparison", ROOT / "scripts/compare_floodgate_holdout_runs.py")
module = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(module)


def document(bestmove="7g7f"):
    return {"diagnostic_only": True, "nodes": 1, "warmup_nodes": 0,
            "execution": {"source_revision": "abc", "binary": {"sha256": "bin"},
                           "weights": None, "options": {"null_move_pruning": True}, "corpus_sha256": "corpus"},
            "results": [{
        "source": {"game_id": "holdout:a"},
        "unrestricted": {"completion": "search_completed", "aborted": False, "bound": "exact",
                          "bestmove": bestmove, "score_cp": 10, "depth": 2, "pv_usi": [bestmove]},
    }]}


def test_compares_only_complete_unlabeled_results():
    candidate = document("2g2f")
    candidate["execution"]["options"]["null_move_pruning"] = False
    result = module.compare(document(), candidate, {"options.null_move_pruning"})
    assert result["counts"]["comparable"] == 1
    assert result["rows"][0]["bestmove_changed"] is True
    assert result["rows"][0]["pv_changed"] is True
    assert result["decision"] == "not_evaluable"
    assert result["source_corpus_sha256"] == "corpus"
    assert result["baseline_corpus_sha256"] == "corpus"
    assert result["candidate_corpus_sha256"] == "corpus"
    assert result["rows"][0]["baseline_search_integrity"]["status"] == "unknown"
    assert result["rows"][0]["candidate_search_integrity"]["status"] == "unknown"


def test_rejects_unlisted_budget_change():
    candidate = document()
    candidate["nodes"] = 2
    result = module.compare(document(), candidate)
    assert result["counts"]["comparable"] == 0
    assert "nodes" in result["rows"][0]["not_comparable_reasons"]


def test_preserves_mismatched_corpus_provenance():
    candidate = document()
    candidate["execution"]["corpus_sha256"] = "other-corpus"
    result = module.compare(document(), candidate)
    assert result["source_corpus_sha256"] is None
    assert result["baseline_corpus_sha256"] == "corpus"
    assert result["candidate_corpus_sha256"] == "other-corpus"


def test_preserves_explicit_search_integrity_evidence():
    baseline = document()
    candidate = document()
    for item in (baseline, candidate):
        item["results"][0]["unrestricted"].update({
            "pv_legal": True,
            "pv_replay_preserves_input": True,
        })
    result = module.compare(baseline, candidate)
    assert result["rows"][0]["baseline_search_integrity"]["status"] == "verified"
    assert result["rows"][0]["candidate_search_integrity"]["status"] == "verified"


if __name__ == "__main__":
    test_compares_only_complete_unlabeled_results()
    test_rejects_unlisted_budget_change()
    test_preserves_mismatched_corpus_provenance()
    print("PASS")
