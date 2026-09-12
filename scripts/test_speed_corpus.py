import json
import unittest
from pathlib import Path

import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from validate_speed_corpus import validate, validate_document

ROOT = Path(__file__).resolve().parents[1]
CORPUS = ROOT / "scripts/fixtures/speed_corpus_v1.json"


class SpeedCorpusTest(unittest.TestCase):
    def test_checked_in_corpus_is_valid(self):
        self.assertEqual(validate(CORPUS), 32)

    def test_rejects_missing_category(self):
        doc = json.loads(CORPUS.read_text(encoding="utf-8"))
        doc["cases"][0]["category"] = "missing"
        with self.assertRaises(ValueError):
            validate_document(doc)

    def test_rejects_duplicate_sfen(self):
        doc = json.loads(CORPUS.read_text(encoding="utf-8"))
        doc["cases"][1]["sfen"] = doc["cases"][0]["sfen"]
        with self.assertRaises(ValueError):
            validate_document(doc)
