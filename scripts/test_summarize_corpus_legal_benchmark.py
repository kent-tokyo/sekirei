import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from summarize_corpus_legal_benchmark import summarize
from test_validate_corpus_legal_benchmark import fixture


class SummarizeCorpusLegalBenchmarkTest(unittest.TestCase):
    def test_summarizes_tuning_and_holdout(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / "output.txt"
            output.write_text(fixture(), encoding="utf-8")
            split = root / "split.json"
            split.write_text(json.dumps({
                "schema": "sekirei.speed-corpus-split.v1",
                "source_corpus_sha256": "a" * 64,
                "tuning_case_ids": [f"case-{i:03d}" for i in range(64)],
                "holdout_case_ids": [f"case-{i:03d}" for i in range(64, 128)],
            }), encoding="utf-8")
            result = summarize(output, split)
            self.assertEqual(result["splits"]["tuning"]["case_count"], 64)
            self.assertEqual(result["splits"]["holdout"]["case_count"], 64)

    def test_rejects_hash_mismatch(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            output = root / "output.txt"
            output.write_text(fixture(), encoding="utf-8")
            split = root / "split.json"
            split.write_text(json.dumps({
                "schema": "sekirei.speed-corpus-split.v1",
                "source_corpus_sha256": "b" * 64,
                "tuning_case_ids": [], "holdout_case_ids": [],
            }), encoding="utf-8")
            with self.assertRaises(ValueError):
                summarize(output, split)


if __name__ == "__main__":
    unittest.main()
