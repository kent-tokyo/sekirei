#!/usr/bin/env python3
"""Focused tests for static NNUE transcript profiling."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("profile", ROOT / "profile_nnue_transcript.py")
assert SPEC and SPEC.loader
PROFILE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PROFILE)


class TranscriptProfileTests(unittest.TestCase):
    def test_summary_reports_output_diversity(self) -> None:
        rows = [
            {"static_score_cp": 10, "attributes": {"phase": "opening", "material_band": "balanced"}},
            {"static_score_cp": 10, "attributes": {"phase": "opening", "material_band": "balanced"}},
            {"static_score_cp": -20, "attributes": {"phase": "middlegame", "material_band": "stm_behind"}},
        ]
        summary = PROFILE.summarize(rows)
        self.assertEqual(summary["distinct_scores"], 2)
        self.assertEqual(summary["dominant_score_cp"], 10)
        self.assertEqual(summary["dominant_score_count"], 2)
        self.assertAlmostEqual(summary["dominant_score_ratio"], 2 / 3)


if __name__ == "__main__":
    unittest.main()
