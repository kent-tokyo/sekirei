#!/usr/bin/env python3
import importlib.util
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("hypotheses", ROOT / "scripts/build_floodgate_hypothesis_table.py")
module = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(module)


def test_builds_non_causal_table():
    document = {
        "schema": "sekirei.floodgate-diagnostic-summary.v1",
        "diagnostic_only": True,
        "claims": {"strength": "not_permitted"},
        "execution": {
            "source_revision": "abc", "binary": {"sha256": "bin"},
            "weights": None, "options": {"threads": 1}, "corpus_sha256": "corpus",
        },
        "nodes": 100, "warmup_nodes": 100,
        "rows": [{
            "diagnostic_class": "incomplete_search",
            "unrestricted_aborted": True,
            "actual_root_aborted": True,
            "unrestricted_pv": [],
            "actual_root_pv": [],
        }],
    }
    result = module.build(document)
    assert result["claims"]["causal_inference"] == "not_proven"
    assert result["source"]["corpus_sha256"] == "corpus"
    assert next(r for r in result["rows"] if r["diagnostic_class"] == "incomplete_search")["observed_rows"] == 1
    assert all(row["status"] == "unknown" for row in result["rows"])


if __name__ == "__main__":
    test_builds_non_causal_table()
    print("PASS")
