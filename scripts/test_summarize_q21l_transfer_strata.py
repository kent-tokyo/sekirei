#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
from pathlib import Path


SCRIPT = Path(__file__).with_name("summarize_q21l_transfer_strata.py")
SPEC = importlib.util.spec_from_file_location("q21l_strata", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def test_depth_and_king_danger_classes_are_explicit() -> None:
    assert MODULE.depth_relation(4, 5) == "candidate_shallower"
    assert MODULE.depth_relation(6, 5) == "candidate_deeper"
    assert MODULE.depth_relation(5, 5) == "same_depth"
    assert MODULE.king_danger({"in_check": True, "legal_checks": 0}) == "in_check"
    assert (
        MODULE.king_danger({"in_check": False, "legal_checks": 2})
        == "checking_option_available"
    )
    assert (
        MODULE.king_danger({"in_check": False, "legal_checks": 0})
        == "no_immediate_check"
    )


if __name__ == "__main__":
    test_depth_and_king_danger_classes_are_explicit()
    print("PASS")
