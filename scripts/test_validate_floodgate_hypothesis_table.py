#!/usr/bin/env python3
import importlib.util
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("validator", ROOT / "scripts/validate_floodgate_hypothesis_table.py")
module = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(module)


def valid():
    return {
        "schema": "sekirei.floodgate-diagnostic-hypotheses.v1",
        "diagnostic_only": True,
        "claims": {"strength": "not_permitted", "causal_inference": "not_proven", "played_move_is_label": False},
        "source": {
            "source_revision": "abc", "binary": {"sha256": "bin"}, "weights": None,
            "options": {"threads": 1}, "corpus_sha256": "corpus",
        },
        "rows": [{
            "diagnostic_class": "incomplete_search", "observed_rows": 4,
            "status": "unknown",
            "hypothesis": "探索不足", "support": "予算増で完了", "reject": "再現しない",
            "next_test": "固定予算比較",
        }],
    }


def test_valid():
    assert module.validate(valid()) == []


def test_rejects_causal_claim():
    document = valid()
    document["claims"]["causal_inference"] = "proven"
    assert "claims.causal_inference" in module.validate(document)


if __name__ == "__main__":
    test_valid()
    test_rejects_causal_claim()
    print("PASS")
