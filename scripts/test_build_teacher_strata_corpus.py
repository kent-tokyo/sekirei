#!/usr/bin/env python3
"""Focused tests for teacher-cache strata selection."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("strata", ROOT / "build_teacher_strata_corpus.py")
assert SPEC and SPEC.loader
STRATA = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(STRATA)


class TeacherStrataTests(unittest.TestCase):
    def test_select_limits_each_stratum_in_sorted_sfen_order(self) -> None:
        rows = [
            {"sfen": "z", "teacher_class": "mate", "phase": "opening", "material_band": "balanced"},
            {"sfen": "a", "teacher_class": "mate", "phase": "opening", "material_band": "balanced"},
            {"sfen": "b", "teacher_class": "non_mate", "phase": "endgame", "material_band": "stm_behind"},
        ]
        selected = STRATA.select(rows, 1)
        self.assertEqual([row["sfen"] for row in selected], ["a", "b"])


if __name__ == "__main__":
    unittest.main()
