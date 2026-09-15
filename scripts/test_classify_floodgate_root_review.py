#!/usr/bin/env python3
import importlib.util
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("classifier", ROOT / "scripts/classify_floodgate_root_review.py")
classifier = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(classifier)


def row(**comparison):
    return {
        "game_id": "g",
        "ply": 1,
        "history_replay": {"status": "verified"},
        "cold": {"completion": "search_completed"},
        "warm": {"completion": "search_completed"},
        "actual_root": {"completion": "search_completed"},
        "comparison": {"bestmove_matches_played": True, "warm_bestmove_changed": False,
                        "warm_score_delta_cp": 0, "actual_score_delta_cp": 0, **comparison},
    }


def test_classifies_observed_categories():
    assert classifier.classify_row(row(actual_score_delta_cp=-450)) == "played_move_root_difference_observed"
    assert classifier.classify_row(row(warm_bestmove_changed=True)) == "tt_bestmove_change_observed"
    assert classifier.classify_row(row(warm_score_delta_cp=25)) == "tt_score_change_observed"
    assert classifier.classify_row(row()) == "no_difference_observed"
    incomplete = row()
    incomplete["cold"]["completion"] = "timeout"
    assert classifier.classify_row(incomplete) == "incomplete_search"


def test_summary_is_diagnostic_only():
    result = classifier.classify({"schema": "sekirei.floodgate-root-review.v1", "rows": [row(), row(actual_score_delta_cp=-1)]})
    assert result["rows"] == 2
    assert result["claims"]["strength"] == "not_permitted"
    assert result["classification_counts"]["no_difference_observed"] == 1
    assert result["classifications"][0]["next_test"]


if __name__ == "__main__":
    test_classifies_observed_categories()
    test_summary_is_diagnostic_only()
    print("PASS")
