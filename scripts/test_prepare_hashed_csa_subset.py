#!/usr/bin/env python3
"""Regression tests for deterministic disjoint CSA subset selection."""
import importlib.util
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("subset", ROOT / "scripts/prepare_hashed_csa_subset.py")
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


def test_offset_selects_disjoint_rank_ranges(tmp_path):
    for name in ("a.csa", "b.csa", "c.csa", "d.csa"):
        (tmp_path / name).write_text("V2.2\nPI\n", encoding="utf-8")
    first = MODULE.choose(tmp_path, 2, 0)
    second = MODULE.choose(tmp_path, 2, 2)
    assert set(first).isdisjoint(second)
    assert first + second == MODULE.choose(tmp_path, 4, 0)


def test_offset_rejects_unavailable_range(tmp_path):
    (tmp_path / "only.csa").write_text("V2.2\nPI\n", encoding="utf-8")
    with pytest.raises(ValueError, match="offset"):
        MODULE.choose(tmp_path, 1, 1)
