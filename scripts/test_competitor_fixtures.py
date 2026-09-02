#!/usr/bin/env python3
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from validate_competitor_corpus import validate_corpus


ROOT = Path(__file__).resolve().parents[1]
CORPUS = ROOT / "scripts" / "competitor_parity_corpus.json"


class CompetitorFixtureTests(unittest.TestCase):
    def test_checked_in_corpus_is_valid(self):
        self.assertEqual(validate_corpus(CORPUS), 6)

    def test_invalid_move_is_rejected(self):
        document = json.loads(CORPUS.read_text(encoding="utf-8"))
        document["cases"][0]["moves"] = ["bad"]
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "invalid.json"
            path.write_text(json.dumps(document), encoding="utf-8")
            with self.assertRaises(ValueError):
                validate_corpus(path)


if __name__ == "__main__":
    unittest.main()
