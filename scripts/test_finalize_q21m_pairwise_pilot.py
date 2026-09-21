#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
from pathlib import Path


SCRIPT = Path(__file__).with_name("finalize_q21m_pairwise_pilot.py")
SPEC = importlib.util.spec_from_file_location("q21m_finalize", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def test_screen_failure_closes_development_and_q20() -> None:
    decision = MODULE.decision_for("screen_fail")
    assert decision == {
        "q21m_completed": True,
        "candidate_rejected": True,
        "development_match_authorized": False,
        "q20_authorized": False,
    }


def test_screen_pass_only_authorizes_development_match() -> None:
    decision = MODULE.decision_for("screen_pass")
    assert decision["q21m_completed"] is False
    assert decision["candidate_rejected"] is False
    assert decision["development_match_authorized"] is True
    assert decision["q20_authorized"] is False


if __name__ == "__main__":
    test_screen_failure_closes_development_and_q20()
    test_screen_pass_only_authorizes_development_match()
    print("PASS")
