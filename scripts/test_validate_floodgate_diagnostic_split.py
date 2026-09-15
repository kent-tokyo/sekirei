#!/usr/bin/env python3
import importlib.util
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("validator", ROOT / "scripts/validate_floodgate_diagnostic_split.py")
module = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(module)


def test_rejects_overlap():
    corpus = {"entries": [{"source": {"game_id": "a"}}, {"source": {"game_id": "b"}}]}
    split = {"schema": module.SCHEMA, "diagnostic_only": True, "source_corpus_sha256": "hash",
             "tuning_entry_indices": [0, 1], "holdout_entry_indices": [1],
             "tuning_games": ["a", "b"], "holdout_games": ["b"]}
    try:
        module.validate_documents(corpus, split, "hash")
    except ValueError as exc:
        assert "disjoint" in str(exc)
    else:
        raise AssertionError("overlapping source games accepted")


if __name__ == "__main__":
    test_rejects_overlap()
    print("PASS")
