#!/usr/bin/env python3
import importlib.util
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("validator", ROOT / "scripts/validate_completed_diagnostic_selection.py")
module = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(module)


def test_valid_empty_selection():
    assert module.validate({
        "schema": "sekirei.floodgate-completed-diagnostic-selection.v1",
        "diagnostic_only": True, "status": "not_evaluable", "rows": [],
        "excluded": [{"index": 0, "reasons": ["aborted"]}],
        "counts": {"selected": 0, "excluded": 1},
        "claims": {"strength": "not_permitted"},
    }) == []


def test_rejects_ready_row_without_exact_completion():
    document = {
        "schema": "sekirei.floodgate-completed-diagnostic-selection.v1",
        "diagnostic_only": True, "status": "ready",
        "rows": [{"diagnostic_class": "no_difference_observed",
                   "unrestricted_aborted": True, "actual_root_aborted": False,
                   "unrestricted_bound": "unknown", "actual_root_bound": "exact"}],
        "excluded": [], "counts": {"selected": 1, "excluded": 0},
        "claims": {"strength": "not_permitted"},
    }
    assert "rows[0].aborted" in module.validate(document)
    assert "rows[0].bound" in module.validate(document)


if __name__ == "__main__":
    test_valid_empty_selection()
    test_rejects_ready_row_without_exact_completion()
    print("PASS")
