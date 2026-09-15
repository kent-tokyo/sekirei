import importlib.util
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("summary", ROOT / "scripts/summarize_gate_pair_outcomes.py")
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


def write_game(path, result, engine1):
    path.write_text(
        f"# Engine1: {engine1}\n# Result: {result}\n"
        "position sfen 9/9/9/9/9/9/9/9/9 b - 1 moves 5i5h\n",
        encoding="utf-8",
    )


def test_pair_summary_reconciles_two_kifus(tmp_path):
    (tmp_path / "shard_0000_kifu").mkdir()
    write_game(tmp_path / "shard_0000_kifu/game0001.txt", "Engine1 Win", "candidate (Black)")
    write_game(tmp_path / "shard_0000_kifu/game0002.txt", "Engine2 Win", "candidate (White)")
    (tmp_path / "shard_0000.json").write_text(json.dumps({"games": 2, "engine1_wins": 1, "engine2_wins": 1, "draws": 0}), encoding="utf-8")
    result = MODULE.summarize(tmp_path)
    assert result["summary"] == {"pairs": 1, "classes": {"split": 1}, "errors": 0}
    assert result["pairs"][0]["pair_id"] == "shard_0000"


def test_pair_summary_rejects_missing_kifu(tmp_path):
    (tmp_path / "shard_0000_kifu").mkdir()
    (tmp_path / "shard_0000.json").write_text(json.dumps({"games": 2}), encoding="utf-8")
    assert MODULE.summarize(tmp_path)["summary"]["errors"] == 1
