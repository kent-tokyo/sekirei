#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
from pathlib import Path


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("screen", ROOT / "evaluate_q21i_imitation_screen.py")
assert SPEC and SPEC.loader
SCREEN = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SCREEN)


def model(loss: float, blunders: int) -> dict:
    return {
        "mean_parent_rank_loss_cp": loss,
        "major_blunders_ge_300cp": blunders,
        "parent_diagnostics": [{"parent_id": "a"}, {"parent_id": "b"}],
    }


def test_pass_requires_both_checks() -> None:
    assert SCREEN.evaluate(model(100.0, 2), model(89.0, 2))["status"] == "screen_pass"
    assert SCREEN.evaluate(model(100.0, 2), model(89.0, 3))["status"] == "screen_fail"
    assert SCREEN.evaluate(model(100.0, 2), model(91.0, 1))["status"] == "screen_fail"


def test_parent_sets_must_match() -> None:
    candidate = model(80.0, 1)
    candidate["parent_diagnostics"][1]["parent_id"] = "c"
    try:
        SCREEN.evaluate(model(100.0, 2), candidate)
    except ValueError:
        return
    raise AssertionError("mismatched parents must fail")


if __name__ == "__main__":
    test_pass_requires_both_checks()
    test_parent_sets_must_match()
    print("PASS")
