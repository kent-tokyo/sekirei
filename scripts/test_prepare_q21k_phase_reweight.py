#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import json
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "q21k_phase", ROOT / "prepare_q21k_phase_reweight.py"
)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def test_phase_weights_equalise_mass_without_changing_mean_scale() -> None:
    counts = {"opening": 2, "middlegame": 4, "endgame": 6}
    total = sum(counts.values())
    weights = {phase: total / (3 * count) for phase, count in counts.items()}
    masses = [counts[phase] * weights[phase] for phase in MODULE.PHASES]
    assert masses[0] == masses[1] == masses[2]
    assert sum(masses) / total == 1.0


if __name__ == "__main__":
    test_phase_weights_equalise_mass_without_changing_mean_scale()
    print("PASS")
