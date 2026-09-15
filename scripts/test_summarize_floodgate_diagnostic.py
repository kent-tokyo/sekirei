#!/usr/bin/env python3
import importlib.util


from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("summary", ROOT / "scripts/summarize_floodgate_diagnostic.py")
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


def test_summary_preserves_diagnostic_boundaries():
    report = {
        "schema": "sekirei.floodgate-core-diagnostic.v2",
        "diagnostic_only": True,
        "results": [{
            "index": 0,
            "source": {"game_id": "g", "ply": 23},
            "observed_move_usi": "3c4e",
            "unrestricted": {"bestmove": "7g7f", "pv_usi": ["7g7f"], "bound": "exact", "aborted": False,
                             "completed_bound": "exact", "completed_iteration_valid": "true",
                             "root_candidates": [{"move": "7g7f", "score_cp": 120}]},
            "actual_root": {"pv_usi": ["3c4e"], "bound": "exact", "aborted": False,
                            "completed_bound": "exact", "completed_iteration_valid": "true"},
            "comparison": {
                "unrestricted_bestmove_matches_played": False,
                "score_delta_actual_root_minus_unrestricted_cp": -120,
                "depth_delta_actual_root_minus_unrestricted": 0,
            },
            "unrestricted_tt_comparison": {
                "bestmove_changed": False,
                "score_delta_warm_minus_cold_cp": 0,
            },
        }],
    }
    summary = MODULE.classify(report)
    assert summary["claims"]["strength"] == "not_permitted"
    assert summary["rows"][0]["score_delta_actual_minus_unrestricted_cp"] == -120
    assert summary["rows"][0]["unrestricted_pv"] == ["7g7f"]
    assert summary["rows"][0]["root_candidates"][0]["move"] == "7g7f"
    assert summary["rows"][0]["diagnostic_class"] == "root_score_gap_observed"


def test_budget_abort_with_exact_completed_pass_is_comparable():
    report = {
        "schema": "sekirei.floodgate-core-diagnostic.v2", "diagnostic_only": True,
        "results": [{
            "unrestricted": {"bound": "unknown", "aborted": True, "completed_bound": "exact",
                               "completed_iteration_valid": "true", "pv_usi": [], "root_candidates": []},
            "actual_root": {"bound": "unknown", "aborted": True, "completed_bound": "exact",
                            "completed_iteration_valid": "true", "pv_usi": []},
            "comparison": {"score_delta_actual_root_minus_unrestricted_cp": 0},
            "unrestricted_tt_comparison": {},
        }],
    }
    assert MODULE.classify(report)["rows"][0]["diagnostic_class"] == "no_difference_observed"


if __name__ == "__main__":
    test_summary_preserves_diagnostic_boundaries()
    test_budget_abort_with_exact_completed_pass_is_comparable()
    print("PASS")
