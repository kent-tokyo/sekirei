#!/usr/bin/env python3
"""Regression tests for deterministic stratified MultiPV selection."""

import unittest

from run_stratified_multipv_probe import choose_rows


SFEN = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1"


def record(sample_id, move, score):
    return {"sample_id": sample_id, "status": "ok", "bestmove": move, "lines": [{"score_cp": score}]}


class StratifiedSelectionTest(unittest.TestCase):
    def test_selection_is_sorted_and_limited_per_phase_bucket(self):
        corpus = {}
        candidate = {}
        teacher = {}
        material = {}
        for index, (ply, candidate_move, teacher_move, material_move) in enumerate(
            ((41, "7g7f", "2g2f", "7g7f"), (81, "7g7f", "2g2f", "3c3d"), (121, "7g7f", "2g2f", "4c4d"), (161, "7g7f", "2g2f", "5c5d"))
        ):
            sample_id = f"sample-{index}"
            sfen = SFEN.rsplit(" ", 1)[0] + f" {ply}"
            corpus[sample_id] = {"sfen": sfen}
            candidate[sample_id] = record(sample_id, candidate_move, 100)
            teacher[sample_id] = record(sample_id, teacher_move, 0)
            material[sample_id] = record(sample_id, material_move, 0)
        selected = choose_rows(corpus, candidate, teacher, material, per_group=1)
        self.assertEqual([row["sample_id"] for row in selected], ["sample-0", "sample-1", "sample-2"])
        self.assertEqual([row["phase"] for row in selected], ["early", "middle", "late"])

    def test_matching_bestmove_is_excluded(self):
        sample_id = "same"
        corpus = {sample_id: {"sfen": SFEN}}
        record_set = record(sample_id, "7g7f", 0)
        selected = choose_rows(corpus, {sample_id: record_set}, {sample_id: record_set}, {sample_id: record_set}, 1)
        self.assertEqual(selected, [])


if __name__ == "__main__":
    unittest.main()
