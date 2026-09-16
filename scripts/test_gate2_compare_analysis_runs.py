#!/usr/bin/env python3
"""Fixture tests for gate2_compare_analysis_runs.py's metric functions.
No subprocess, no real engine -- synthetic analysis_record_v1-shaped
dicts fed straight into the metric functions, matching this repo's
existing test-file convention (see scripts/test_usi_analysis_export.py):
stdlib unittest only.

Run: python3 scripts/test_gate2_compare_analysis_runs.py
"""
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from gate2_compare_analysis_runs import (
    compare,
    coverage,
    mate_agreement,
    score_cp_shift,
    top1_agreement,
    top3_overlap,
)


def _rec(sample_id, bestmove="7g7f", status="ok", lines=None):
    return {
        "sample_id": sample_id,
        "bestmove": bestmove if status == "ok" else None,
        "status": status,
        "lines": lines if lines is not None else ([_line(1, bestmove, 10)] if status == "ok" else []),
    }


def _line(multipv, bestmove, score_cp=None, score_mate=None):
    line = {"multipv": multipv, "bestmove": bestmove, "pv": [bestmove]}
    if score_mate is not None:
        line["score_mate"] = score_mate
    else:
        line["score_cp"] = score_cp if score_cp is not None else 0
    return line


class Top1AgreementTest(unittest.TestCase):
    def test_all_agree(self):
        we = {"a": _rec("a", "7g7f"), "b": _rec("b", "2g2f")}
        woe = {"a": _rec("a", "7g7f"), "b": _rec("b", "2g2f")}
        r = top1_agreement(we, woe, {"a", "b"})
        self.assertEqual(r, {"n": 2, "agreement": 1.0})

    def test_partial_agreement(self):
        we = {"a": _rec("a", "7g7f"), "b": _rec("b", "2g2f")}
        woe = {"a": _rec("a", "7g7f"), "b": _rec("b", "3c3d")}
        r = top1_agreement(we, woe, {"a", "b"})
        self.assertEqual(r["n"], 2)
        self.assertAlmostEqual(r["agreement"], 0.5)

    def test_excludes_non_ok_bestmove_none(self):
        # A status!=ok position has bestmove=None on both sides -- must
        # not be counted as a spurious "agreement" (None == None).
        we = {"a": _rec("a", status="timeout")}
        woe = {"a": _rec("a", status="timeout")}
        r = top1_agreement(we, woe, {"a"})
        self.assertEqual(r, {"n": 0, "agreement": None})


class Top3OverlapTest(unittest.TestCase):
    def test_identical_top3_sets(self):
        lines = [_line(1, "7g7f", 10), _line(2, "2g2f", 5), _line(3, "3c3d", 0)]
        we = {"a": _rec("a", lines=lines)}
        woe = {"a": _rec("a", lines=lines)}
        r = top3_overlap(we, woe, {"a"})
        self.assertEqual(r["n"], 1)
        self.assertEqual(r["mean_jaccard"], 1.0)
        self.assertEqual(r["exact_match_rate"], 1.0)

    def test_disjoint_top3_sets(self):
        we_lines = [_line(1, "7g7f", 10), _line(2, "2g2f", 5), _line(3, "3c3d", 0)]
        woe_lines = [_line(1, "1g1f", 10), _line(2, "9g9f", 5), _line(3, "5g5f", 0)]
        we = {"a": _rec("a", lines=we_lines)}
        woe = {"a": _rec("a", lines=woe_lines)}
        r = top3_overlap(we, woe, {"a"})
        self.assertEqual(r["mean_jaccard"], 0.0)
        self.assertEqual(r["exact_match_rate"], 0.0)

    def test_truncates_to_first_three_by_multipv(self):
        # A 4th line must not affect the comparison.
        we_lines = [_line(1, "7g7f"), _line(2, "2g2f"), _line(3, "3c3d"), _line(4, "9g9f")]
        woe_lines = [_line(1, "7g7f"), _line(2, "2g2f"), _line(3, "3c3d")]
        we = {"a": _rec("a", lines=we_lines)}
        woe = {"a": _rec("a", lines=woe_lines)}
        r = top3_overlap(we, woe, {"a"})
        self.assertEqual(r["mean_jaccard"], 1.0)

    def test_excludes_non_ok_positions(self):
        we = {"a": _rec("a", status="timeout")}
        woe = {"a": _rec("a", status="timeout")}
        r = top3_overlap(we, woe, {"a"})
        self.assertEqual(r, {"n": 0, "mean_jaccard": None, "exact_match_rate": None})


class ScoreCpShiftTest(unittest.TestCase):
    def test_computes_within_engine_delta(self):
        we = {"a": _rec("a", lines=[_line(1, "7g7f", score_cp=120)])}
        woe = {"a": _rec("a", lines=[_line(1, "7g7f", score_cp=100)])}
        r = score_cp_shift(we, woe, {"a"})
        self.assertEqual(r["n"], 1)
        self.assertEqual(r["mean"], 20)
        self.assertEqual(r["median"], 20)

    def test_excludes_mate_lines_from_cp_delta(self):
        we = {"a": _rec("a", lines=[_line(1, "7g7f", score_mate=3)])}
        woe = {"a": _rec("a", lines=[_line(1, "7g7f", score_cp=100)])}
        r = score_cp_shift(we, woe, {"a"})
        self.assertEqual(r["n"], 0)
        self.assertEqual(r["excluded_mate"], 1)

    def test_empty_yields_none_not_a_crash(self):
        r = score_cp_shift({}, {}, set())
        self.assertEqual(r["mean"], None)


class MateAgreementTest(unittest.TestCase):
    def test_agrees_on_matching_mate_scores(self):
        we = {"a": _rec("a", lines=[_line(1, "7g7f", score_mate=3)])}
        woe = {"a": _rec("a", lines=[_line(1, "7g7f", score_mate=3)])}
        r = mate_agreement(we, woe, {"a"})
        self.assertEqual(r, {"n": 1, "agreement": 1.0})

    def test_never_compares_mate_against_cp(self):
        we = {"a": _rec("a", lines=[_line(1, "7g7f", score_mate=3)])}
        woe = {"a": _rec("a", lines=[_line(1, "7g7f", score_cp=9000)])}
        r = mate_agreement(we, woe, {"a"})
        self.assertEqual(r, {"n": 0, "agreement": None})


class CoverageTest(unittest.TestCase):
    def test_breaks_down_by_status(self):
        recs = {
            "a": _rec("a", status="ok"),
            "b": _rec("b", status="ok"),
            "c": _rec("c", status="timeout"),
            "d": _rec("d", status="engine_error"),
        }
        r = coverage(recs, "test")
        self.assertEqual(r["total"], 4)
        self.assertEqual(r["counts"], {"ok": 2, "timeout": 1, "incomplete": 0, "engine_error": 1})
        self.assertAlmostEqual(r["non_ok_rate"], 0.5)


class CompareEndToEndTest(unittest.TestCase):
    def test_full_pipeline_on_small_synthetic_corpus(self):
        import json
        import tempfile

        we_records = [
            _rec("a", "7g7f", lines=[_line(1, "7g7f", 50), _line(2, "2g2f", 10), _line(3, "3c3d", -5)]),
            _rec("b", "1g1f", lines=[_line(1, "1g1f", -30)]),
        ]
        woe_records = [
            _rec("a", "7g7f", lines=[_line(1, "7g7f", 30), _line(2, "3c3d", 5), _line(3, "9g9f", -10)]),
            _rec("b", "9g9f", lines=[_line(1, "9g9f", -10)]),
        ]
        with tempfile.NamedTemporaryFile(mode="w", suffix=".jsonl", delete=False) as f1, \
                tempfile.NamedTemporaryFile(mode="w", suffix=".jsonl", delete=False) as f2:
            for r in we_records:
                f1.write(json.dumps(r) + "\n")
            for r in woe_records:
                f2.write(json.dumps(r) + "\n")
            f1_path, f2_path = f1.name, f2.name
        try:
            result = compare(f1_path, f2_path)
            self.assertEqual(result["corpus_size"]["shared"], 2)
            self.assertEqual(result["top1_agreement"]["n"], 2)
            self.assertAlmostEqual(result["top1_agreement"]["agreement"], 0.5)  # a agrees, b disagrees
        finally:
            os.unlink(f1_path)
            os.unlink(f2_path)

    def test_raises_on_duplicate_sample_id_within_one_file(self):
        import json
        import tempfile

        with tempfile.NamedTemporaryFile(mode="w", suffix=".jsonl", delete=False) as f:
            f.write(json.dumps(_rec("a", "7g7f")) + "\n")
            f.write(json.dumps(_rec("a", "2g2f")) + "\n")
            path = f.name
        try:
            with self.assertRaises(ValueError):
                compare(path, path)
        finally:
            os.unlink(path)


if __name__ == "__main__":
    unittest.main()
