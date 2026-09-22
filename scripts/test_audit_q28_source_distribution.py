#!/usr/bin/env python3
"""Focused tests for Q28's score-free source-distribution audit."""

from __future__ import annotations

import importlib.util
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("q28", ROOT / "audit_q28_source_distribution.py")
assert SPEC and SPEC.loader
Q28 = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(Q28)


def facts(**overrides: object) -> dict[str, int | bool]:
    result: dict[str, int | bool] = {
        "legal_moves": 30,
        "in_check": False,
        "mate_in_one": False,
        "capture_moves": 0,
        "checking_moves": 0,
        "forcing_moves": 0,
    }
    result.update(overrides)
    return result


class Q28SourceDistributionTests(unittest.TestCase):
    def test_tactical_class_priority_is_disjoint(self) -> None:
        self.assertEqual(Q28.tactical_class(facts()), "quiet")
        self.assertEqual(Q28.tactical_class(facts(capture_moves=2, forcing_moves=2)), "capture_resource")
        self.assertEqual(Q28.tactical_class(facts(checking_moves=1, forcing_moves=1)), "checking_resource")
        self.assertEqual(
            Q28.tactical_class(facts(in_check=True, capture_moves=2, checking_moves=1, forcing_moves=3)),
            "evasion",
        )

    def test_parse_rejects_inconsistent_tactical_counts(self) -> None:
        with self.assertRaises(ValueError):
            Q28.parse_fact_line(
                "legal_moves=2\tin_check=false\tmate_in_one=false\tcapture_moves=3\tchecking_moves=0\tforcing_moves=2"
            )

    def test_summary_keeps_phase_material_and_scores_out_of_contract(self) -> None:
        positions = [
            {"id": "a", "category": "opening/balanced"},
            {"id": "b", "category": "endgame/stm_behind"},
        ]
        summary = Q28.summarize(
            positions,
            [facts(), facts(capture_moves=1, forcing_moves=1)],
        )
        self.assertEqual(summary["tactical_class_counts"], {"capture_resource": 1, "quiet": 1})
        self.assertEqual(summary["by_phase_material"]["opening/balanced"], {"quiet": 1})
        self.assertNotIn("score", str(summary).lower())


if __name__ == "__main__":
    unittest.main()
