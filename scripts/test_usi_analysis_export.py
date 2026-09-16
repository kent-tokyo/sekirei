#!/usr/bin/env python3
"""Fixture tests for usi_analysis_export.py's USI line parser and record
builder. No subprocess, no real engine -- saved USI output strings are
fed straight into the parser functions, matching this repo's existing
test-file convention (see scripts/test_gate_resource_preflight.py):
stdlib unittest only.

Run: python3 scripts/test_usi_analysis_export.py
"""
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from usi_analysis_export import build_record, classify_status, parse_bestmove_line, parse_info_line


class ParseInfoLineTest(unittest.TestCase):
    def test_score_cp(self):
        r = parse_info_line("info depth 12 score cp 34 nodes 5000 nps 100000 time 50 hashfull 10 pv 7g7f")
        self.assertEqual(r["score_cp"], 34)
        self.assertNotIn("score_mate", r)
        self.assertEqual(r["pv"], ["7g7f"])
        self.assertEqual(r["depth"], 12)
        self.assertEqual(r["nodes"], 5000)
        self.assertEqual(r["nps"], 100000)
        self.assertEqual(r["time_ms"], 50)

    def test_score_mate_multimove_pv(self):
        r = parse_info_line("info depth 20 score mate 3 nodes 900 nps 5000 time 10 pv 7g7f 3c3d 2g2f")
        self.assertEqual(r["score_mate"], 3)
        self.assertNotIn("score_cp", r)
        self.assertEqual(r["pv"], ["7g7f", "3c3d", "2g2f"])

    def test_bound_lowerbound(self):
        r = parse_info_line("info depth 8 score cp 120 lowerbound nodes 10 nps 10 time 1 pv 2g2f")
        self.assertEqual(r["bound"], "lowerbound")

    def test_bound_upperbound(self):
        r = parse_info_line("info depth 8 score cp -50 upperbound nodes 10 nps 10 time 1 pv 2g2f")
        self.assertEqual(r["bound"], "upperbound")

    def test_multipv_1_2_3(self):
        lines = [
            "info multipv 1 depth 10 score cp 50 nodes 100 nps 10 time 1 pv 7g7f",
            "info multipv 2 depth 10 score cp 30 nodes 100 nps 10 time 1 pv 2g2f",
            "info multipv 3 depth 10 score cp 10 nodes 100 nps 10 time 1 pv 6g6f",
        ]
        ranks = [parse_info_line(line)["multipv"] for line in lines]
        self.assertEqual(ranks, [1, 2, 3])

    def test_bestmove_with_ponder(self):
        r = parse_bestmove_line("bestmove 7g7f ponder 3c3d")
        self.assertEqual(r, {"bestmove": "7g7f", "ponder": "3c3d"})

    def test_bestmove_resign(self):
        r = parse_bestmove_line("bestmove resign")
        self.assertEqual(r, {"bestmove": "resign", "ponder": None})

    def test_sekirei_line_missing_optional_fields(self):
        # Real single-PV Sekirei output: no multipv/seldepth/bound tokens.
        r = parse_info_line("info depth 13 score cp 88 nodes 500000 nps 1100000 time 450 hashfull 30 pv 2g2f")
        self.assertNotIn("multipv", r)
        self.assertNotIn("seldepth", r)
        self.assertNotIn("bound", r)
        self.assertEqual(r["depth"], 13)
        self.assertEqual(r["score_cp"], 88)
        self.assertEqual(r["pv"], ["2g2f"])

    def test_info_string_ignored(self):
        self.assertIsNone(parse_info_line("info string NNUE weights loaded from data/foo.bin"))
        self.assertIsNone(parse_info_line("info string book move"))

    def test_truncated_line_returns_none(self):
        self.assertIsNone(parse_info_line("info depth 5 nodes 10 nps 1 time 1"))

    def test_pv_token_with_nothing_after(self):
        r = parse_info_line("info depth 1 score cp 0 nodes 1 nps 1 time 1 pv")
        self.assertEqual(r["pv"], [])


class BuildRecordTest(unittest.TestCase):
    def test_timeout_record_never_dropped(self):
        record = build_record(
            sample_id="game1:12",
            game_id="game1",
            ply=12,
            sfen="lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1",
            engine_info={"name": "sekirei", "version": "0.3.5", "build_info": None,
                         "binary_sha256": "a" * 64, "weight_sha256": None},
            settings_info={"threads": 1, "hash_mb": 64, "multipv": 1, "depth": 8},
            status="timeout",
            lines=[],
            bestmove=None,
            ponder=None,
            error_detail="timed out after 60s waiting for bestmove",
            wall_time_ms=60000,
        )
        for key in ("schema_version", "sample_id", "game_id", "ply", "sfen", "engine",
                    "settings", "lines", "status", "error_detail", "wall_time_ms", "bestmove"):
            self.assertIn(key, record)
        self.assertEqual(record["lines"], [])
        self.assertIsNone(record["bestmove"])
        self.assertIsNotNone(record["error_detail"])
        self.assertEqual(record["status"], "timeout")
        self.assertNotIn("ponder", record)  # optional key, omitted when None


class ClassifyStatusTest(unittest.TestCase):
    def test_ok(self):
        self.assertEqual(
            classify_status(timed_out=False, saw_bestmove=True, crashed=False, have_lines=True), "ok"
        )

    def test_timeout_wins_over_everything(self):
        self.assertEqual(
            classify_status(timed_out=True, saw_bestmove=True, crashed=True, have_lines=True), "timeout"
        )

    def test_incomplete_when_bestmove_but_no_lines(self):
        # e.g. a book-move short-circuit -- see module docstring.
        self.assertEqual(
            classify_status(timed_out=False, saw_bestmove=True, crashed=False, have_lines=False),
            "incomplete",
        )

    def test_engine_error_when_no_bestmove(self):
        self.assertEqual(
            classify_status(timed_out=False, saw_bestmove=False, crashed=False, have_lines=False),
            "engine_error",
        )


if __name__ == "__main__":
    unittest.main()
