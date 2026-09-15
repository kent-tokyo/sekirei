import importlib.util
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
SPEC = importlib.util.spec_from_file_location("corpus", ROOT / "scripts/build_floodgate_diagnostic_corpus.py")
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


def test_fixture_is_safe_observation_corpus():
    csa = ROOT / "scripts/fixtures/analysis_replay_v1.csa"
    analysis = ROOT / "scripts/fixtures/analysis_replay_v1.analysis.jsonl"
    entries = MODULE.extract(csa, analysis)
    assert entries == []


def test_record_selection_adds_deterministic_nonterminal_controls():
    records = [
        {"ply": 1, "sign_reversal": True, "score_kind": "cp", "actual_move_csa": "+7776FU"},
        {"ply": 2, "sign_reversal": False, "score_kind": "cp", "actual_move_csa": "-3334FU"},
        {"ply": 3, "sign_reversal": False, "score_kind": "mate", "actual_move_csa": "+2726FU"},
        {"ply": 4, "sign_reversal": False, "score_kind": "cp", "actual_move_csa": "+2625FU"},
    ]
    selected = MODULE._select_records(records, limit=1, controls_per_game=1)
    assert [(record["ply"], kind) for record, kind in selected] == [(1, "swing"), (2, "control")]


def test_local_four_game_corpus_has_no_invalid_pairs_when_available():
    csa_dir = ROOT / "data/floodgate"
    analysis_dir = ROOT / "data/floodgate-analysis"
    if not csa_dir.is_dir() or not analysis_dir.is_dir():
        return
    document = MODULE.build(csa_dir, analysis_dir, 200, None)
    assert document["invalid_games"] == []
    assert document["diagnostic_only"] is True
    assert document["strength_claim"] == "not_permitted"
    assert all(entry["label_policy"] == "observation_only_no_correct_move_label" for entry in document["entries"])


if __name__ == "__main__":
    test_fixture_is_safe_observation_corpus()
    test_local_four_game_corpus_has_no_invalid_pairs_when_available()
    print("PASS")
