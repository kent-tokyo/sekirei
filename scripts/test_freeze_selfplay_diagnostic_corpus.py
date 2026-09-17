#!/usr/bin/env python3
"""Small contract test for ``freeze_selfplay_diagnostic_corpus.py``."""

from __future__ import annotations

import importlib.util
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("freeze", ROOT / "freeze_selfplay_diagnostic_corpus.py")
assert SPEC and SPEC.loader
FREEZE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(FREEZE)


def row(number: int, category: str, opening: str) -> dict:
    return {
        "source_id": "healthy", "game_number": number, "ply": number, "opening": opening,
        "pre_move_sfen": "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
        "history_before_usi": [], "actual_move_csa": "+7776FU", "categories": [category],
        "already_in_teacher_cache": False,
    }


def main() -> int:
    rows = []
    number = 0
    for category, quota in FREEZE.CATEGORY_QUOTAS.items():
        for _ in range(quota):
            number += 1
            rows.append(row(number, category, f"opening-{number}"))
    chosen = FREEZE.choose(rows, Path("fixture.json"))
    assert len(chosen) == 12
    assert {item["selection_category"] for item in chosen} == set(FREEZE.CATEGORY_QUOTAS)
    rows[0]["already_in_teacher_cache"] = True
    try:
        FREEZE.choose(rows, Path("fixture.json"))
    except ValueError as error:
        assert "teacher-cache-overlapping" in str(error)
    else:
        raise AssertionError("teacher overlap must fail closed")
    print("freeze selfplay diagnostic corpus: ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
