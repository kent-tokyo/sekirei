#!/usr/bin/env python3
import importlib.util


ROOT = __import__("pathlib").Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("comparison", ROOT / "scripts/compare_floodgate_diagnostic_summaries.py")
module = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(module)


def summary(rows):
    return {
        "schema": "sekirei.floodgate-diagnostic-summary.v1",
        "diagnostic_only": True,
        "claims": {"strength": "not_permitted"},
        "execution": {
            "source_revision": "abc",
            "binary": {"sha256": "bin"}, "weights": None,
            "options": {"threads": 1, "spec_top_n": 0}, "corpus_sha256": "corpus",
        },
        "nodes": 100, "warmup_nodes": 100,
        "rows": rows,
    }


def row(game_id="g", ply=3, complete=True):
    return {
        "game_id": game_id, "ply": ply,
        "diagnostic_class": "no_difference_observed" if complete else "incomplete_search",
        "unrestricted_aborted": not complete, "actual_root_aborted": not complete,
        "unrestricted_pv": [], "actual_root_pv": [],
        "unrestricted_bound": "exact" if complete else "unknown",
        "actual_root_bound": "exact" if complete else "unknown",
        "unrestricted_bestmove": "7g7f",
        "score_delta_actual_minus_unrestricted_cp": 0,
    }


def test_matches_by_stable_key_and_quarantines_incomplete():
    result = module.compare(summary([row(), row("h", 4, False)]), summary([row("h", 4, False), row()]))
    assert result["counts"] == {"total": 2, "comparable": 1, "not_comparable": 1}
    assert result["decision"] == "not_evaluable"
    assert result["source_corpus_sha256"] == "corpus"


def test_rejects_duplicate_keys():
    try:
        module.compare(summary([row(), row()]), summary([row()]))
    except ValueError as exc:
        assert "duplicate" in str(exc)
    else:
        raise AssertionError("duplicate key accepted")


def test_rejects_key_mismatch():
    try:
        module.compare(summary([row()]), summary([row("other")]))
    except ValueError as exc:
        assert "mismatch" in str(exc)
    else:
        raise AssertionError("key mismatch accepted")


def test_allows_only_declared_single_ablation_change():
    baseline = summary([row()])
    candidate = summary([row()])
    candidate["execution"]["source_revision"] = "candidate"
    candidate["execution"]["binary"]["sha256"] = "candidate-bin"
    result = module.compare(baseline, candidate, {"source_revision", "binary.sha256"})
    assert result["counts"]["comparable"] == 1
    candidate["nodes"] = 101
    rejected = module.compare(baseline, candidate, {"source_revision", "binary.sha256"})
    assert rejected["counts"]["comparable"] == 0
    assert "nodes" in rejected["rows"][0]["not_comparable_reasons"]


def test_reports_root_candidate_set_and_score_differences_as_diagnostics():
    baseline = row()
    baseline["root_candidates"] = [
        {"move": "7g7f", "score_cp": 10, "depth": 2, "bound": "exact", "abort_reason": "none"},
        {"move": "2g2f", "score_cp": 5, "depth": 2, "bound": "exact", "abort_reason": "none"},
    ]
    candidate = row()
    candidate["root_candidates"] = [
        {"move": "7g7f", "score_cp": 14, "depth": 2, "bound": "exact", "abort_reason": "none"},
        {"move": "2g2f", "score_cp": 5, "depth": 2, "bound": "exact", "abort_reason": "none"},
    ]
    result = module.compare(summary([baseline]), summary([candidate]))
    compared = result["rows"][0]
    assert compared["root_candidate_set_changed"] is False
    assert compared["root_candidate_scores_changed"] is True
    assert compared["root_candidate_score_deltas_cp"] == {"2g2f": 0, "7g7f": 4}


def test_preserves_mismatched_corpus_provenance():
    baseline = summary([row()])
    candidate = summary([row()])
    candidate["execution"]["corpus_sha256"] = "other-corpus"
    result = module.compare(baseline, candidate)
    assert result["source_corpus_sha256"] is None
    assert result["baseline_corpus_sha256"] == "corpus"
    assert result["candidate_corpus_sha256"] == "other-corpus"


if __name__ == "__main__":
    test_matches_by_stable_key_and_quarantines_incomplete()
    test_rejects_duplicate_keys()
    test_rejects_key_mismatch()
    test_allows_only_declared_single_ablation_change()
    test_reports_root_candidate_set_and_score_differences_as_diagnostics()
    test_preserves_mismatched_corpus_provenance()
    print("PASS")
