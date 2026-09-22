#!/usr/bin/env python3
"""Unit tests for Q25d score-blind rule eligibility."""

from __future__ import annotations

import unittest
from pathlib import Path
from unittest.mock import patch

import prepare_q25a_external_label_family as preparer


class RuleEligibilityTests(unittest.TestCase):
    def test_filters_only_predeclared_rule_facts_and_records_them(self) -> None:
        candidates = {stratum: [] for stratum in preparer.q21w.STRATA}
        candidates[preparer.q21w.STRATA[0]] = [
            {"category": preparer.q21w.STRATA[0], "sfen": "first"},
            {"category": preparer.q21w.STRATA[0], "sfen": "second"},
        ]
        facts = [
            {"legal_moves": 2, "in_check": False, "mate_in_one": False},
            {"legal_moves": 1, "in_check": True, "mate_in_one": False},
        ]
        with patch.object(preparer, "rule_facts", return_value=facts):
            retained, rejected = preparer.apply_rule_eligibility(
                candidates,
                Path("rule-probe"),
                {"min_legal_moves": 2, "exclude_mate_in_one": True},
            )
        self.assertEqual(len(retained[preparer.q21w.STRATA[0]]), 1)
        self.assertEqual(rejected[preparer.q21w.STRATA[0]], 1)
        self.assertEqual(
            retained[preparer.q21w.STRATA[0]][0]["rule_eligibility"], facts[0]
        )

    def test_contract_never_allows_a_one_move_parent(self) -> None:
        with self.assertRaisesRegex(ValueError, "at least two"):
            preparer.apply_rule_eligibility(
                {stratum: [] for stratum in preparer.q21w.STRATA},
                Path("rule-probe"),
                {"min_legal_moves": 1},
            )

    def test_can_exclude_checked_parents_without_reading_scores(self) -> None:
        candidates = {stratum: [] for stratum in preparer.q21w.STRATA}
        candidates[preparer.q21w.STRATA[0]] = [
            {"category": preparer.q21w.STRATA[0], "sfen": "checked"},
        ]
        with patch.object(
            preparer,
            "rule_facts",
            return_value=[{"legal_moves": 4, "in_check": True, "mate_in_one": False}],
        ):
            retained, rejected = preparer.apply_rule_eligibility(
                candidates,
                Path("rule-probe"),
                {"min_legal_moves": 2, "exclude_in_check": True},
            )
        self.assertEqual(retained[preparer.q21w.STRATA[0]], [])
        self.assertEqual(rejected[preparer.q21w.STRATA[0]], 1)


if __name__ == "__main__":
    unittest.main()
