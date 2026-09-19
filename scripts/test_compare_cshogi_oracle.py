#!/usr/bin/env python3
"""Schema checks for the optional cshogi legal-move oracle."""

from __future__ import annotations

import json
import tempfile
from pathlib import Path

from compare_cshogi_oracle import expected_matches, load_cases


ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "scripts" / "fixtures" / "cshogi_boundary_cases_v1.json"


def test_versioned_fixture_loads_and_has_independent_expectations() -> None:
    cases = load_cases(FIXTURE)
    assert len(cases) == 7
    assert all("source" in case and "rule" in case for case in cases)
    exact = next(case for case in cases if case["id"] == "double-check-king-only")
    assert expected_matches(exact, ["5i4i", "5i6h", "5i6i"])
    assert not expected_matches(exact, ["5i4i"])
    counted = next(case for case in cases if case["id"] == "pawn-drop-mate-boundary")
    assert expected_matches(counted, ["x"] * 129)
    assert not expected_matches(counted, ["x"] * 128)


def test_rejects_unsorted_expected_move_set() -> None:
    document = {
        "schema": "sekirei.cshogi-boundary-cases.v1",
        "cases": [{
            "id": "bad",
            "initial_sfen": "9/9/9/9/9/9/9/9/9 b - 1",
            "history_usi": [],
            "expected_legal_usi": ["7g7f", "2g2f"],
        }],
    }
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "bad.json"
        path.write_text(json.dumps(document), encoding="utf-8")
        try:
            load_cases(path)
        except ValueError as error:
            assert "sorted" in str(error)
        else:
            raise AssertionError("unsorted expected move set was accepted")


if __name__ == "__main__":
    test_versioned_fixture_loads_and_has_independent_expectations()
    test_rejects_unsorted_expected_move_set()
    print("PASS")
