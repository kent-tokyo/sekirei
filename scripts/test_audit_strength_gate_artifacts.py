#!/usr/bin/env python3
"""Small fixture tests for strength-gate evidence classification."""

from __future__ import annotations

import importlib.util
import json
import tempfile
from pathlib import Path


SCRIPT = Path(__file__).with_name("audit_strength_gate_artifacts.py")
SPEC = importlib.util.spec_from_file_location("audit_strength_gate_artifacts", SCRIPT)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def write(path: Path, value: object) -> None:
    path.write_text(json.dumps(value) + "\n", encoding="utf-8")


def formal_plan() -> dict[str, object]:
    return {
        "evaluation": {
            "candidate": {"mode": "residual-material"},
            "baseline": {"mode": "residual-material"},
        },
        "protocol": {
            "games_per_position": 2,
            "positions": 200,
            "max_games": 400,
            "sprt": {
                "elo0": 0,
                "elo1": 20,
                "alpha": 0.05,
                "beta": 0.05,
                "variant": "trinomial",
                "paired_by_id": True,
            },
        }
    }


with tempfile.TemporaryDirectory() as temp:
    run = Path(temp)
    write(run / "plan.json", formal_plan())
    for name in ("execution.json", "state.json", "preflight.json"):
        write(run / name, {"ok": True})
    (run / "combined.jsonl").write_text(
        '{"id":"p0","result":"candidate_win"}\n{"id":"p0","result":"baseline_win"}\n',
        encoding="utf-8",
    )
    assert MODULE.audit(run)["formal_eligibility"] is True
    legacy = formal_plan()
    legacy["protocol"]["games_per_position"] = 4
    write(run / "plan.json", legacy)
    document = MODULE.audit(run)
    assert document["formal_eligibility"] is False
    assert "games_per_position_not_two" in document["ineligibility_reasons"]
print("PASS")
