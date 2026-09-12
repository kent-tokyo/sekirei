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
        self.assertEqual(validate(CORPUS), 128)

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

    def test_rejects_missing_required_case(self):
        doc = json.loads(CORPUS.read_text(encoding="utf-8"))
        doc["required_case_ids"] = doc["required_case_ids"][:-1]
        with self.assertRaises(ValueError):
            validate_document(doc)

    def test_rejects_case_id_drift(self):
        doc = json.loads(CORPUS.read_text(encoding="utf-8"))
        doc["cases"][0]["id"] = "renamed-case"
        with self.assertRaises(ValueError):
            validate_document(doc)

    def test_rejects_missing_numeric_expectations(self):
        doc = json.loads(CORPUS.read_text(encoding="utf-8"))
        del doc["cases"][0]["expected"]["perft2"]
        with self.assertRaises(ValueError):
            validate_document(doc)

    def test_rejects_incomplete_legal_move_set(self):
        doc = json.loads(CORPUS.read_text(encoding="utf-8"))
        doc["cases"][0]["expected"]["legal_moves_usi"] = []
        with self.assertRaises(ValueError):
            validate_document(doc)

    def test_rejects_incomplete_perft_divide(self):
        doc = json.loads(CORPUS.read_text(encoding="utf-8"))
        doc["cases"][0]["expected"]["perft2_divide"] = []
        with self.assertRaises(ValueError):
            validate_document(doc)

    def test_rejects_perft_divide_total_mismatch(self):
        doc = json.loads(CORPUS.read_text(encoding="utf-8"))
        doc["cases"][0]["expected"]["perft2_divide"][0] = "1a1b:0"
        with self.assertRaises(ValueError):
            validate_document(doc)

    def test_rejects_corpus_hash_drift(self):
        doc = json.loads(CORPUS.read_text(encoding="utf-8"))
        doc["cases"][0]["source"] = "changed"
        with self.assertRaises(ValueError):
            validate_document(doc)
