#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
from pathlib import Path


SCRIPT = Path(__file__).with_name("prepare_q21m_pairwise_pilot.py")
SPEC = importlib.util.spec_from_file_location("q21m_prepare", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def row(index: int, phase: str, material: str) -> dict:
    return {
        "sfen": f"board b - {index + 1}",
        "source": {"source_key": f"g{index}"},
        "tags": {"phase": phase, "material_band": material},
    }


def test_score_blind_selection_is_balanced_and_deterministic() -> None:
    rows = [
        row(index * 10 + repeat, phase, material)
        for index, (phase, material) in enumerate(
            (p, m) for p in MODULE.PHASES for m in MODULE.MATERIAL_BANDS
        )
        for repeat in range(3)
    ]
    first = MODULE.select(rows, 2, 4343)
    second = MODULE.select(list(reversed(rows)), 2, 4343)
    assert [item["sfen"] for item in first] == [item["sfen"] for item in second]
    assert len(first) == 18
    assert {MODULE.stratum(item) for item in first} == set(MODULE.STRATA)


if __name__ == "__main__":
    test_score_blind_selection_is_balanced_and_deterministic()
    print("PASS")
