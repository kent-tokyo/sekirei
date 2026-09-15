#!/usr/bin/env python3
import importlib.util
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("splitter", ROOT / "scripts/split_floodgate_diagnostic_corpus.py")
module = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(module)


def entry(game):
    return {"source": {"game_id": game}}


def test_splits_whole_source_games():
    result = module.split({"schema": "corpus", "entries": [entry("b"), entry("a"), entry("b")]}, "hash")
    assert result["tuning_games"] == ["a"]
    assert result["holdout_games"] == ["b"]
    assert result["tuning_entry_indices"] == [1]
    assert result["holdout_entry_indices"] == [0, 2]


def test_rejects_single_source_game():
    try:
        module.split({"entries": [entry("a")]}, "hash")
    except ValueError as exc:
        assert "two source games" in str(exc)
    else:
        raise AssertionError("single source game accepted")


if __name__ == "__main__":
    test_splits_whole_source_games()
    test_rejects_single_source_game()
    print("PASS")
