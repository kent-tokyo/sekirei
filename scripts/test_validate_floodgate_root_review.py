#!/usr/bin/env python3
import importlib.util
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("validator", ROOT / "scripts/validate_floodgate_root_review.py")
validator = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(validator)


def valid_document():
    return {
        "schema": "sekirei.floodgate-root-review.v1",
        "diagnostic_only": True,
        "strength_claim": "not_permitted",
        "contract": {"nodes": 20_000, "max_depth": 2, "threads": 1, "spec_top_n": 0, "played_move_is_label": False},
        "binary": {"path": "engine", "sha256": "a" * 64},
        "history_binary": {"path": "history", "sha256": "b" * 64},
        "rows": [{
            "game_id": "game", "result": "win", "ply": 2, "played_move_usi": "7g7f",
            "played_move_is_label": False,
            "source": {"csa": "game.csa", "analysis": "game.jsonl", "csa_sha256": "c" * 64, "analysis_sha256": "d" * 64},
            "history_replay": {"status": "verified"},
            "cold": {"completion": "search_completed"},
            "warm": {"completion": "search_completed"},
            "actual_root": {"completion": "search_completed"},
            "comparison": {"requested_node_budget": 20_000, "same_requested_budget": True, "bestmove_matches_played": True, "warm_bestmove_changed": False, "warm_score_delta_cp": 0, "actual_score_delta_cp": 0},
        }],
    }


def test_accepts_complete_diagnostic_document():
    assert validator.validate(valid_document()) == []


def test_rejects_strength_claim_and_incomplete_row():
    document = valid_document()
    document["strength_claim"] = "elo"
    document["rows"][0]["history_replay"]["status"] = "timeout"
    errors = validator.validate(document)
    assert "strength_claim" in errors
    assert "rows[0].history_replay" in errors


def test_v2_requires_explicit_requested_budget_contract():
    document = valid_document()
    document["schema"] = "sekirei.floodgate-root-review.v2"
    errors = validator.validate(document)
    assert "rows[0].comparison.requested_node_budget" in errors
    assert "rows[0].comparison.same_requested_budget" in errors


def test_v2_accepts_complete_search_arm_fields():
    document = valid_document()
    document["schema"] = "sekirei.floodgate-root-review.v2"
    for name in ("cold", "warm"):
        document["rows"][0][name] = {
            "bestmove": "7g7f", "depth": 1, "score_cp": 0, "nodes": 20_000,
            "completion": "search_completed",
        }
    document["rows"][0]["actual_root"] = {
        "bestmove": "7g7f", "root_move_usi": "7g7f", "depth": 1,
        "score_cp": 0, "nodes": 20_000, "completion": "search_completed",
    }
    document["rows"][0]["comparison"].update({
        "requested_node_budget": 20_000, "same_requested_budget": True,
    })
    assert validator.validate(document) == []


if __name__ == "__main__":
    test_accepts_complete_diagnostic_document()
    test_rejects_strength_claim_and_incomplete_row()
    print("PASS")
