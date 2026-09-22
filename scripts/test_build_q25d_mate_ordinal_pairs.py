#!/usr/bin/env python3
"""Unit tests for Q25d's explicit mate-aware pair boundary."""

from __future__ import annotations

import unittest
import tempfile
import hashlib
import json
from pathlib import Path

import build_q25d_mate_ordinal_pairs as builder


def row(scores: dict[str, dict[str, int]]) -> dict[str, object]:
    return {
        "id": "parent",
        "labels": {
            move: {"status": "labeled", "score": score}
            for move, score in scores.items()
        },
    }


class MateOrdinalPairTests(unittest.TestCase):
    def test_positive_mates_prefer_shorter_distance_without_cp_gap(self) -> None:
        pairs, audit = builder.adjacent_pairs(
            row({"7g7f": {"kind": "mate", "value": 3}, "2g2f": {"kind": "mate", "value": 7}})
        )
        self.assertFalse(audit["excluded"])
        self.assertEqual(pairs[0]["higher_move_usi"], "7g7f")
        self.assertEqual(pairs[0]["label_kind"], "mate_ordinal")
        self.assertNotIn("teacher_score_gap_cp", pairs[0])

    def test_negative_mates_prefer_longer_distance_without_cp_gap(self) -> None:
        pairs, _ = builder.adjacent_pairs(
            row({"7g7f": {"kind": "mate", "value": -4}, "2g2f": {"kind": "mate", "value": -2}})
        )
        self.assertEqual(pairs[0]["higher_move_usi"], "7g7f")

    def test_mixed_cp_mate_and_mixed_signs_are_not_silently_ranked(self) -> None:
        _, mixed_kind = builder.adjacent_pairs(
            row({"7g7f": {"kind": "cp", "value": 10}, "2g2f": {"kind": "mate", "value": 3}})
        )
        _, mixed_sign = builder.adjacent_pairs(
            row({"7g7f": {"kind": "mate", "value": 3}, "2g2f": {"kind": "mate", "value": -3}})
        )
        self.assertEqual(mixed_kind["reason"], "mixed_cp_and_mate_scores_without_two_cp_moves")
        self.assertEqual(mixed_sign["reason"], "mixed_mate_signs")

    def test_mixed_root_keeps_only_a_strict_cp_subspace(self) -> None:
        pairs, audit = builder.adjacent_pairs(
            row({
                "7g7f": {"kind": "cp", "value": 20},
                "2g2f": {"kind": "cp", "value": -10},
                "3g3f": {"kind": "mate", "value": 3},
            })
        )
        self.assertFalse(audit["excluded"])
        self.assertEqual(audit["omitted_mate_moves"], ["3g3f"])
        self.assertEqual(pairs[0]["label_kind"], "centipawn")

    def test_mixed_terminal_ordinal_is_opt_in_and_does_not_invent_cp_gap(self) -> None:
        pairs, audit = builder.adjacent_pairs(
            row({"7g7f": {"kind": "mate", "value": 3}, "2g2f": {"kind": "cp", "value": 100}}),
            allow_mixed_terminal_ordinal=True,
        )
        self.assertFalse(audit["excluded"])
        self.assertTrue(audit["mixed_terminal_ordinal"])
        self.assertEqual(pairs[0]["higher_move_usi"], "7g7f")
        self.assertEqual(pairs[0]["label_kind"], "terminal_ordinal")
        self.assertNotIn("teacher_score_gap_cp", pairs[0])

    def test_mixed_terminal_ordinal_omits_out_of_range_cp_without_using_it(self) -> None:
        pairs, audit = builder.adjacent_pairs(
            row({
                "7g7f": {"kind": "mate", "value": -6},
                "2g2f": {"kind": "mate", "value": -8},
                "3g3f": {"kind": "cp", "value": 20_001},
            }),
            allow_mixed_terminal_ordinal=True,
        )
        self.assertFalse(audit["excluded"])
        self.assertEqual(audit["omitted_out_of_range_cp_moves"], ["3g3f"])
        self.assertEqual(pairs[0]["label_kind"], "mate_ordinal")

    def test_family_pair_semantics_requires_the_bound_sha(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            family = Path(directory) / "family.json"
            family.write_text(json.dumps({
                "schema": "sekirei.q25a-external-label-family-preregistration.v1",
                "status": "frozen_before_score_blind_parent_selection",
                "pair_semantics": {"allow_mixed_terminal_ordinal": True},
            }), encoding="utf-8")
            prereg = {"family": {"path": str(family), "sha256": hashlib.sha256(family.read_bytes()).hexdigest()}}
            self.assertTrue(builder.frozen_pair_semantics(prereg)["allow_mixed_terminal_ordinal"])
            prereg["family"]["sha256"] = "0" * 64
            with self.assertRaisesRegex(ValueError, "SHA"):
                builder.frozen_pair_semantics(prereg)


if __name__ == "__main__":
    unittest.main()
