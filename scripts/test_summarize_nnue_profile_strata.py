#!/usr/bin/env python3
"""Focused tests for root-profile strata metrics."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("strata", ROOT / "summarize_nnue_profile_strata.py")
assert SPEC and SPEC.loader
STRATA = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(STRATA)


class RootProfileStrataTests(unittest.TestCase):
    def test_groups_phase_material_and_mate_class(self) -> None:
        row = {
            "comparable": True,
            "bestmove_changed": True,
            "attributes": {"phase": "opening", "material_band": "balanced"},
            "baseline": {"score_cp": 100},
            "candidate": {"score_cp": -50},
            "teacher": {"score_cp": -80},
        }
        summary = STRATA.summarize_rows([row])
        self.assertEqual(summary["phase"]["opening"]["n"], 1)
        self.assertEqual(summary["material_band"]["balanced"]["candidate_same_sign"], 1)
        self.assertEqual(summary["teacher_class"]["non_mate"]["baseline_abs_error_mean_cp"], 180.0)

    def test_frozen_label_overrides_research_mate_class(self) -> None:
        row = {
            "sfen": "fixture", "comparable": True, "bestmove_changed": False,
            "attributes": {"phase": "endgame", "material_band": "black_ahead"},
            "baseline": {"score_cp": 0}, "candidate": {"score_cp": 0},
            "teacher": {"score_cp": 0},
        }
        summary = STRATA.summarize_rows([row], {"fixture": 899_000})
        self.assertEqual(summary["teacher_class"]["mate"]["n"], 1)


if __name__ == "__main__":
    unittest.main()
