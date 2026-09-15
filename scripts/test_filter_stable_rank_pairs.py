#!/usr/bin/env python3
import importlib.util
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("stable", ROOT / "scripts/filter_stable_rank_pairs.py")
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


def document(pairs):
    return {"schema": "sekirei.root-rank-pairs.v1", "diagnostic_only": True, "pairs": pairs}


def pair(parent, high, low):
    return {"parent_id": parent, "higher_move_usi": high, "lower_move_usi": low}


def test_retains_only_orders_stable_at_deeper_teacher():
    output, manifest = MODULE.filter_pairs(
        document([pair("p", "7g7f", "2g2f"), pair("p", "2g2f", "7g7f")]),
        document([pair("p", "7g7f", "2g2f")]),
    )
    assert output["pairs"] == [pair("p", "7g7f", "2g2f")]
    assert manifest["retained_pairs"] == 1
    assert manifest["output_sha256"] == MODULE.digest(output)


def test_rejects_no_stable_order():
    with pytest.raises(ValueError, match="no pair"):
        MODULE.filter_pairs(document([pair("p", "7g7f", "2g2f")]), document([]))
