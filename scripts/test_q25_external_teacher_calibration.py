#!/usr/bin/env python3
"""Unit tests for Q25 parsing, overlap extraction, and decision metrics."""

from __future__ import annotations

import unittest
from pathlib import Path
from tempfile import TemporaryDirectory

import finalize_q25_external_teacher_calibration as finalizer
import prepare_q25_external_teacher_calibration as preparer
import preregister_q25_external_label_family as family
import run_q25_external_teacher_calibration as runner


class Q25Tests(unittest.TestCase):
    def test_sfen_detection_is_strict_enough_for_recursive_evidence(self) -> None:
        self.assertTrue(preparer.looks_like_sfen("9/9/9/9/9/9/9/9/9 b - 1"))
        self.assertFalse(preparer.looks_like_sfen("data/csa/2025/example.csa"))

    def test_plain_sfen_files_are_supported(self) -> None:
        with TemporaryDirectory() as directory:
            path = Path(directory) / "positions.sfen"
            path.write_text("9/9/9/9/9/9/9/9/9 b - 1\n", encoding="utf-8")
            self.assertEqual(list(preparer.documents(path)), ["9/9/9/9/9/9/9/9/9 b - 1"])

    def test_external_cp_and_mate_lines_are_parsed(self) -> None:
        cp = runner.parse_info("info depth 10 multipv 2 score cp -31 nodes 10 time 1 pv 7g7f")
        mate = runner.parse_info("info depth 10 score mate + nodes 12 time 1 pv G*5b")
        self.assertEqual(cp["score"], {"kind": "cp", "value": -31})
        self.assertEqual(mate["score"], {"kind": "mate", "value": 1, "symbolic": "+"})

    def test_yaneuraou_depth_precedes_searchmoves(self) -> None:
        self.assertEqual(runner.go_command(10, None), "go depth 10")
        self.assertEqual(runner.go_command(10, "P*8c"), "go depth 10 searchmoves P*8c")
        self.assertEqual(
            runner.go_command(None, "P*8c", nodes=50_000),
            "go nodes 50000 searchmoves P*8c",
        )

    def test_cp_regret_and_mate_class_are_separate(self) -> None:
        self.assertEqual(
            finalizer.direct_regret(
                {"kind": "cp", "value": 250}, {"kind": "cp", "value": -100}
            ),
            (350, False),
        )
        self.assertEqual(
            finalizer.direct_regret(
                {"kind": "mate", "value": 3}, {"kind": "cp", "value": 900}
            ),
            (None, True),
        )
        self.assertEqual(
            finalizer.direct_regret(
                {"kind": "mate", "value": 3}, {"kind": "mate", "value": 5}
            ),
            (None, False),
        )

    def test_aa_stability_checks_move_and_score(self) -> None:
        rows = [
            {"completion": "search_completed", "bestmove": "7g7f", "score_cp": 20},
            {"completion": "search_completed", "bestmove": "7g7f", "score_cp": 20},
        ]
        self.assertTrue(finalizer.stable(rows, finalizer.self_score))
        rows[1]["score_cp"] = 21
        self.assertFalse(finalizer.stable(rows, finalizer.self_score))

    def test_external_score_for_move_uses_frozen_multipv(self) -> None:
        result = {
            "lines": [
                {"move": "3f5h", "score": {"kind": "cp", "value": 2597}},
                {"move": "P*8c", "score": {"kind": "cp", "value": 2174}},
            ]
        }
        self.assertEqual(
            finalizer.external_score_for_move(result, "P*8c"),
            {"kind": "cp", "value": 2174},
        )

    def test_external_family_schema_is_stable(self) -> None:
        self.assertEqual(
            family.SCHEMA,
            "sekirei.q25a-external-label-family-preregistration.v1",
        )


if __name__ == "__main__":
    unittest.main()
