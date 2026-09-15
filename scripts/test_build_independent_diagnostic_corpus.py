#!/usr/bin/env python3
import importlib.util
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("builder", ROOT / "scripts/build_independent_diagnostic_corpus.py")
module = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(module)


def test_builds_unlabeled_entries():
    result = module.build({"positions": [{"id": "a", "sfen": "9/9/9/9/9/9/9/9/9 b - 1"}]}, "hash")
    assert result["split"] == "independent_holdout_only"
    assert result["entries"][0]["label_policy"].startswith("unlabeled")
    assert result["entries"][0]["position"]["actual_move_observed"] is None


def test_excludes_unmaterialized_move_sequences():
    result = module.build({"positions": [{"id": "a", "moves": ["7g7f"]}]}, "hash")
    assert result["entries"] == []
    assert result["excluded_positions"][0]["reason"].startswith("SFEN")


if __name__ == "__main__":
    test_builds_unlabeled_entries()
    test_excludes_unmaterialized_move_sequences()
    print("PASS")
