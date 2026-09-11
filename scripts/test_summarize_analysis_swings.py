#!/usr/bin/env python3
import importlib.util
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).parent
SPEC = importlib.util.spec_from_file_location("summarize_analysis_swings", ROOT / "summarize_analysis_swings.py")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class SwingSummaryTest(unittest.TestCase):
    def test_groups_results_and_ranks_negative_swings(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "report.json"
            path.write_text(
                '{"schema":"sekirei.analysis-swing-report.v1","game_id":"g1","result":"lose","bestmove_mismatches":1,"alignment_errors":[],"records":[{"ply":2,"score_delta_cp":-350,"swing":true,"terminal":false,"phase":"opening","sfen":"s","actual_move_csa":"+2726FU","bestmove_csa":"+7776FU"}]}',
                encoding="utf-8",
            )
            summary = MODULE.summarize([path])
            self.assertEqual(summary["by_result"]["lose"]["worst_drop_cp"], -350)
            self.assertEqual(summary["top_negative_swings"][0]["ply"], 2)

    def test_rejects_unknown_schema(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "bad.json"
            path.write_text('{"schema":"wrong"}', encoding="utf-8")
            with self.assertRaises(ValueError):
                MODULE.summarize([path])


if __name__ == "__main__":
    unittest.main()
