import importlib.util
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("analyzer", ROOT / "scripts/analyze_gate_pair_classes.py")
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


def test_groups_forcing_classes_by_verified_pair_result():
    corpus = {"diagnostic_only": True, "entries": [{
        "source": {"game_id": "g", "ply": 3, "pair_id": "shard_0000", "outcome": "candidate_loss"},
        "position": {"sfen": "s"},
        "selection_reason": "middle",
    }]}
    classification = {"diagnostic_only": True, "entries": [{"id": MODULE.diagnostic_id(corpus["entries"][0]), "forcing_class": "forced_defense"}]}
    pairs = {"pairs": [{"pair_id": "shard_0000", "pair_class": "candidate_sweep_loss"}]}
    result = MODULE.rows(corpus, classification, pairs)
    assert MODULE.summarize(result) == {"candidate_sweep_loss": {
        "positions": 1, "forcing_classes": {"forced_defense": 1}, "game_outcomes": {"candidate_loss": 1},
    }}


def test_rejects_unknown_pair():
    try:
        corpus = {"diagnostic_only": True, "entries": [{"source": {"game_id": "g", "ply": 0, "pair_id": "x"}, "position": {"sfen": "s"}}]}
        MODULE.rows(corpus,
                    {"diagnostic_only": True, "entries": [{"id": MODULE.diagnostic_id(corpus["entries"][0]), "forcing_class": "quiet"}]},
                    {"pairs": []})
    except ValueError as exc:
        assert "unknown pair" in str(exc)
    else:
        raise AssertionError("unknown pair was accepted")
