#!/usr/bin/env python3
import importlib.util
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("selector", ROOT / "scripts/select_completed_diagnostic_rows.py")
module = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(module)


def summary(aborted=False, bound="exact"):
    return {
        "schema": "sekirei.floodgate-diagnostic-summary.v1", "diagnostic_only": True,
        "claims": {"strength": "not_permitted"}, "execution": {
            "source_revision": "x", "binary": {"sha256": "x"}, "weights": None,
            "options": {}, "corpus_sha256": "x"}, "nodes": 100, "warmup_nodes": 0,
        "rows": [{"index": 0, "diagnostic_class": "no_difference_observed",
                   "unrestricted_aborted": aborted, "actual_root_aborted": aborted,
                   "unrestricted_bound": bound, "actual_root_bound": bound,
                   "unrestricted_pv": [], "actual_root_pv": []}]
    }


def test_selects_exact_complete_row():
    result = module.select(summary())
    assert result["status"] == "ready"
    assert result["counts"] == {"selected": 1, "excluded": 0}


def test_excludes_aborted_row():
    result = module.select(summary(aborted=True))
    assert result["status"] == "not_evaluable"
    assert result["excluded"][0]["reasons"] == ["aborted"]


if __name__ == "__main__":
    test_selects_exact_complete_row()
    test_excludes_aborted_row()
    print("PASS")
