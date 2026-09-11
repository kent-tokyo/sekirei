#!/usr/bin/env python3
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).parent
SPEC = importlib.util.spec_from_file_location("analyze_floodgate_analysis", ROOT / "analyze_floodgate_analysis.py")
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class FloodgateBatchTest(unittest.TestCase):
    def test_pairs_fixture_and_reports_missing_sidecar(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            csa_dir, analysis_dir = root / "csa", root / "analysis"
            csa_dir.mkdir()
            analysis_dir.mkdir()
            fixture = ROOT / "fixtures"
            (csa_dir / "paired.csa").write_text((fixture / "analysis_replay_v1.csa").read_text(), encoding="utf-8")
            (analysis_dir / "paired.analysis.jsonl").write_text((fixture / "analysis_replay_v1.analysis.jsonl").read_text(), encoding="utf-8")
            (csa_dir / "missing.csa").write_text("V2.2\n", encoding="utf-8")
            result = MODULE.analyze_directory(csa_dir, analysis_dir)
            self.assertEqual(result["games_total"], 2)
            self.assertEqual(result["paired"], 1)
            self.assertEqual(len(result["missing_analysis"]), 1)
            self.assertEqual(result["by_result"]["win"]["games"], 1)

    def test_classifies_legacy_sidecar_as_excluded(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            csa_dir, analysis_dir = root / "csa", root / "analysis"
            csa_dir.mkdir()
            analysis_dir.mkdir()
            (csa_dir / "bad.csa").write_text("V2.2\n", encoding="utf-8")
            (analysis_dir / "bad.analysis.jsonl").write_text("{}\n", encoding="utf-8")
            result = MODULE.analyze_directory(csa_dir, analysis_dir)
            self.assertEqual(result["paired"], 0)
            self.assertEqual(len(result["legacy_analysis"]), 1)

    def test_explicitly_excludes_legacy_sidecar(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            csa_dir, analysis_dir = root / "csa", root / "analysis"
            csa_dir.mkdir()
            analysis_dir.mkdir()
            (csa_dir / "legacy.csa").write_text("V2.2\n", encoding="utf-8")
            (analysis_dir / "legacy.analysis.jsonl").write_text(
                '{"schema":"sekirei.analysis-record.v0"}\n', encoding="utf-8"
            )
            result = MODULE.analyze_directory(csa_dir, analysis_dir)
            self.assertEqual(result["paired"], 0)
            self.assertEqual(len(result["legacy_analysis"]), 1)
            self.assertEqual(result["legacy_analysis"][0]["reason"], "legacy_or_unknown_schema")


if __name__ == "__main__":
    unittest.main()
