#!/usr/bin/env python3

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import run_q21g_teacher_judgment_audit as subject


def result(move: str, score: int, elapsed: int = 10):
    return {
        "bestmove": move,
        "score_cp": score,
        "score_kind": subject.score_kind(score),
        "elapsed_ms": elapsed,
        "completed_iteration_valid": True,
        "completed_bound": "exact",
        "pv_legal": True,
        "history_matches_expected": True,
    }


def row(identifier: str, t_move: str, t_score: int, m_move: str, m_score: int):
    return {
        "id": identifier,
        "selection_class": "normal",
        "attributes": {"phase": "opening", "material_band": "balanced"},
        "search": {
            "20000": {
                "B": {"free": result("7g7f", 5)},
                "T": {"free": result(t_move, t_score)},
                "M": {"free": result(m_move, m_score)},
            }
        },
    }


class Q21gAuditTests(unittest.TestCase):
    def test_exact_rejects_non_exact_or_illegal(self):
        self.assertTrue(subject.exact(result("7g7f", 0)))
        bounded = result("7g7f", 0)
        bounded["completed_bound"] = "lower"
        self.assertFalse(subject.exact(bounded))
        illegal = result("7g7f", 0)
        illegal["pv_legal"] = False
        self.assertFalse(subject.exact(illegal))

    def test_deep_selection_prioritizes_teacher_material_disagreement(self):
        disagree = row("disagree", "7g7f", 20, "2g2f", -500)
        agree = row("agree", "7g7f", 20, "7g7f", 21)
        selected = subject.select_deep([agree, disagree], 1)
        self.assertEqual(selected, ["disagree"])

    def test_pair_metrics_are_normal_cp_only(self):
        rows = [row("a", "7g7f", 100, "2g2f", -100), row("b", "7g7f", 40, "7g7f", 10)]
        metrics = subject.pair_metrics(rows, "M", "T")
        self.assertEqual(metrics["overall"]["positions"], 2)
        self.assertEqual(metrics["overall"]["mae_cp"], 115)
        self.assertEqual(metrics["overall"]["sign_mismatches"], 1)
        self.assertEqual(metrics["overall"]["bestmove_mismatches"], 1)
        self.assertFalse(metrics["reference_is_ground_truth"])

    def test_score_kind_separates_mate(self):
        self.assertEqual(subject.score_kind(899_999), "mate")
        self.assertEqual(subject.score_kind(-899_999), "mate")
        self.assertEqual(subject.score_kind(1_200), "cp")


if __name__ == "__main__":
    unittest.main()
