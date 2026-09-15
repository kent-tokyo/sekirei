import importlib.util
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "builder", ROOT / "scripts/build_csa_replay_diagnostic_corpus.py"
)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


def test_v2_replay_keeps_initial_position_and_usi_history(tmp_path):
    document = {
        "schema": "sekirei.csa-replay.v2",
        "player_color": "black",
        "game_id": "g",
        "initial_sfen": "initial",
        "positions": [{
            "ply": 1,
            "pre_move_sfen": "final",
            "history_before": ["+7776FU"],
            "history_before_usi": ["7g7f"],
            "actual_move_csa": "-3334FU",
            "is_player_to_move": True,
            "side_to_move_in_check": False,
        }],
    }
    source = tmp_path / "game.json"
    source.write_text("{}", encoding="utf-8")
    rows, error = MODULE.select_game(document, source, 1)
    assert error is None
    assert rows[0]["position"]["initial_sfen"] == "initial"
    assert rows[0]["position"]["history_before_usi"] == ["7g7f"]


def test_v1_replay_remains_readable_without_claiming_core_history_replay(tmp_path):
    document = {
        "schema": "sekirei.csa-replay.v1",
        "player_color": "black",
        "positions": [{
            "ply": 0,
            "pre_move_sfen": "final",
            "actual_move_csa": "+7776FU",
            "is_player_to_move": True,
            "side_to_move_in_check": False,
        }],
    }
    source = tmp_path / "game.json"
    source.write_text("{}", encoding="utf-8")
    rows, error = MODULE.select_game(document, source, 1)
    assert error is None
    assert rows[0]["position"]["initial_sfen"] is None
    assert rows[0]["position"]["history_before_usi"] == []
