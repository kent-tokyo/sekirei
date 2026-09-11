#!/usr/bin/env python3
import importlib.util
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).parent
SPEC = importlib.util.spec_from_file_location("analyze_analysis_record", ROOT / "analyze_analysis_record.py")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class AnalysisReplayTest(unittest.TestCase):
    def test_aligns_moves_and_flags_swing(self):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            csa = directory / "game.csa"
            csa.write_text("V2.2\n$EVENT:test\nPI\n+7776FU\n-3334FU\n+2726FU\n", encoding="utf-8")
            analysis = directory / "game.analysis.jsonl"
            analysis.write_text(
                "{\"schema\":\"sekirei.analysis-record.v1\",\"engine\":\"sekirei\",\"engine_version\":\"0.3.33\",\"score_perspective\":\"side_to_move\",\"game_id\":\"test\",\"user\":\"sekirei\",\"color\":\"black\"}\n"
                "{\"type\":\"search\",\"ply\":0,\"sfen\":\"9/9/9/9/9/9/9/9/9 b - 1\",\"side_to_move\":\"black\",\"our_color\":\"black\",\"bestmove_csa\":\"+7776FU\",\"score_cp\":100,\"depth\":4,\"nodes\":10,\"elapsed_ms\":1,\"hashfull\":0}\n"
                "{\"type\":\"search\",\"ply\":2,\"sfen\":\"9/9/9/9/9/9/9/9/9 b - 2\",\"side_to_move\":\"black\",\"our_color\":\"black\",\"bestmove_csa\":\"+2726FU\",\"score_cp\":-150,\"depth\":4,\"nodes\":20,\"elapsed_ms\":2,\"hashfull\":0}\n",
                encoding="utf-8",
            )
            report = MODULE.analyze(csa, analysis, threshold=200)
            self.assertEqual(report["alignment_errors"], [])
            self.assertEqual(report["bestmove_mismatches"], 0)
            self.assertEqual(report["swings"], 1)
            self.assertEqual(report["negative_swings"], 1)
            self.assertEqual(report["positive_swings"], 0)
            self.assertEqual(report["records"][1]["actual_move_csa"], "+2726FU")
            self.assertEqual(report["records"][1]["phase"], "middlegame")
            self.assertEqual(report["records"][1]["depth_band"], "shallow")
            self.assertEqual(report["records"][1]["elapsed_band"], "fast")
            self.assertEqual(report["records"][1]["previous_ply"], 0)
            self.assertEqual(report["records"][1]["previous_score_cp"], 100)

    def test_allows_terminal_resignation_record(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            csa = root / "game.csa"
            csa.write_text("$EVENT:test\n+7776FU\n#LOSE\n", encoding="utf-8")
            analysis = root / "game.analysis.jsonl"
            analysis.write_text(
                "{\"schema\":\"sekirei.analysis-record.v1\",\"engine\":\"sekirei\",\"engine_version\":\"0.3.33\",\"score_perspective\":\"side_to_move\",\"game_id\":\"test\",\"user\":\"sekirei\",\"color\":\"black\"}\n"
                "{\"type\":\"search\",\"ply\":1,\"sfen\":\"9/9/9/9/9/9/9/9/9 b - 1\",\"side_to_move\":\"black\",\"our_color\":\"black\",\"bestmove_csa\":null,\"score_cp\":-900000,\"depth\":1,\"nodes\":1,\"elapsed_ms\":1,\"hashfull\":0}\n"
                "{\"type\":\"game_end\",\"result\":\"lose\"}\n",
                encoding="utf-8",
            )
            report = MODULE.analyze(csa, analysis)
            self.assertEqual(report["alignment_errors"], [])
            self.assertTrue(report["records"][0]["terminal"])
            self.assertEqual(report["records"][0]["terminal_reason"], "mate_score")
            self.assertFalse(report["records"][0]["swing"])
            self.assertEqual(report["terminal_records"], 1)

    def test_excludes_mate_score_from_normal_swings(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            csa = root / "game.csa"
            csa.write_text("$EVENT:test\n+7776FU\n-3334FU\n", encoding="utf-8")
            analysis = root / "game.analysis.jsonl"
            analysis.write_text(
                '{"schema":"sekirei.analysis-record.v1","engine":"sekirei","engine_version":"0.3.33","score_perspective":"side_to_move","game_id":"test","user":"sekirei","color":"black"}\n'
                '{"type":"search","ply":0,"sfen":"9/9/9/9/9/9/9/9/9 b - 1","side_to_move":"black","our_color":"black","bestmove_csa":"+7776FU","score_cp":0,"depth":4,"nodes":1,"elapsed_ms":1,"hashfull":0}\n'
                '{"type":"search","ply":2,"sfen":"9/9/9/9/9/9/9/9/9 b - 2","side_to_move":"black","our_color":"black","bestmove_csa":"+2726FU","score_cp":900000,"depth":4,"nodes":1,"elapsed_ms":1,"hashfull":0}\n',
                encoding="utf-8",
            )
            report = MODULE.analyze(csa, analysis)
            self.assertEqual(report["swings"], 0)
            self.assertEqual(report["terminal_records"], 1)

    def test_reports_missing_ply(self):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            csa = directory / "game.csa"
            csa.write_text("+7776FU\n", encoding="utf-8")
            analysis = directory / "game.analysis.jsonl"
            analysis.write_text(
                "{\"schema\":\"sekirei.analysis-record.v1\",\"engine\":\"sekirei\",\"engine_version\":\"0.3.33\",\"score_perspective\":\"side_to_move\",\"game_id\":\"test\",\"user\":\"sekirei\",\"color\":\"black\"}\n"
                "{\"type\":\"search\",\"ply\":3,\"score_cp\":0}\n",
                encoding="utf-8",
            )
            self.assertEqual(len(MODULE.analyze(csa, analysis)["alignment_errors"]), 1)

    def test_rejects_side_mismatch(self):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            csa = directory / "game.csa"
            csa.write_text("+7776FU\n", encoding="utf-8")
            analysis = directory / "game.analysis.jsonl"
            analysis.write_text(
                "{\"schema\":\"sekirei.analysis-record.v1\",\"engine\":\"sekirei\",\"engine_version\":\"0.3.33\",\"score_perspective\":\"side_to_move\",\"game_id\":\"test\",\"user\":\"sekirei\",\"color\":\"black\"}\n"
                "{\"type\":\"search\",\"ply\":0,\"sfen\":\"9/9/9/9/9/9/9/9/9 b - 1\",\"side_to_move\":\"white\",\"our_color\":\"black\",\"bestmove_csa\":\"-7776FU\",\"score_cp\":0,\"depth\":1,\"nodes\":1,\"elapsed_ms\":1,\"hashfull\":0}\n",
                encoding="utf-8",
            )
            errors = MODULE.analyze(csa, analysis)["alignment_errors"]
            self.assertIn("analysis ply 0: side_to_move does not match CSA", errors)
            self.assertIn("analysis ply 0: bestmove color does not match CSA", errors)

    def test_rejects_game_id_mismatch(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            csa = root / "game.csa"
            csa.write_text("$EVENT:actual\n+7776FU\n", encoding="utf-8")
            analysis = root / "game.analysis.jsonl"
            analysis.write_text(
                "{\"schema\":\"sekirei.analysis-record.v1\",\"engine\":\"sekirei\",\"engine_version\":\"0.3.33\",\"score_perspective\":\"side_to_move\",\"game_id\":\"other\",\"user\":\"sekirei\",\"color\":\"black\"}\n",
                encoding="utf-8",
            )
            self.assertIn("CSA $EVENT does not match analysis game_id", MODULE.analyze(csa, analysis)["alignment_errors"])

    def test_rejects_result_mismatch(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            csa = root / "game.csa"
            csa.write_text("$EVENT:test\n#LOSE\n", encoding="utf-8")
            analysis = root / "game.analysis.jsonl"
            analysis.write_text(
                "{\"schema\":\"sekirei.analysis-record.v1\",\"engine\":\"sekirei\",\"engine_version\":\"0.3.33\",\"score_perspective\":\"side_to_move\",\"game_id\":\"test\",\"user\":\"sekirei\",\"color\":\"black\"}\n"
                "{\"type\":\"game_end\",\"result\":\"win\"}\n",
                encoding="utf-8",
            )
            self.assertIn("CSA result does not match analysis game_end", MODULE.analyze(csa, analysis)["alignment_errors"])

    def test_normalizes_legacy_jishogi_result(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            csa = root / "game.csa"
            csa.write_text("$EVENT:test\n#JISHOGI\n", encoding="utf-8")
            analysis = root / "game.analysis.jsonl"
            analysis.write_text(
                '{"schema":"sekirei.analysis-record.v1","engine":"sekirei","engine_version":"0.3.33","score_perspective":"side_to_move","game_id":"test","user":"sekirei","color":"black"}\n'
                '{"type":"game_end","result":"aborted"}\n',
                encoding="utf-8",
            )
            report = MODULE.analyze(csa, analysis)
            self.assertEqual(report["result"], "draw")
            self.assertEqual(report["recorded_result"], "aborted")
            self.assertEqual(report["csa_result"], "draw")
            self.assertEqual(report["alignment_errors"], [])


if __name__ == "__main__":
    unittest.main()
