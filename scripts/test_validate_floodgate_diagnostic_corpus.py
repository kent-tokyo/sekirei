#!/usr/bin/env python3
"""Tests for the diagnostic corpus validator."""
from __future__ import annotations

import copy
import json
import tempfile
import unittest
from pathlib import Path

from validate_floodgate_diagnostic_corpus import validate


ROOT = Path(__file__).resolve().parents[1]
CORPUS = ROOT / "results/floodgate/20260912-review/diagnostic-corpus-with-controls.json"


class DiagnosticCorpusTests(unittest.TestCase):
    def setUp(self) -> None:
        self.document = json.loads(CORPUS.read_text(encoding="utf-8"))

    def test_generated_corpus_and_sources_are_valid(self) -> None:
        self.assertEqual(validate(self.document, root=ROOT, verify_sources=True), [])

    def test_duplicate_source_ply_is_rejected(self) -> None:
        document = copy.deepcopy(self.document)
        document["entries"][1]["source"]["game_id"] = document["entries"][0]["source"]["game_id"]
        document["entries"][1]["source"]["ply"] = document["entries"][0]["source"]["ply"]
        self.assertIn("entries[1].duplicate_source_ply", validate(document))

    def test_label_and_kind_contract_is_rejected(self) -> None:
        document = copy.deepcopy(self.document)
        document["entries"][0]["entry_kind"] = "correct_move"
        document["entries"][0]["label_policy"] = "teacher"
        errors = validate(document)
        self.assertIn("entries[0].entry_kind", errors)
        self.assertIn("entries[0].label_policy", errors)

    def test_source_hash_change_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "source"
            path.write_text("changed", encoding="utf-8")
            document = copy.deepcopy(self.document)
            document["entries"][0]["source"]["csa"] = str(path)
            self.assertIn("entries[0].source.csa_hash", validate(
                document, root=Path("/"), verify_sources=True
            ))

    def test_position_hash_and_history_contract_are_rejected(self) -> None:
        document = copy.deepcopy(self.document)
        document["entries"][0]["position"]["sfen_sha256"] = "0" * 64
        document["entries"][0]["position"]["moves_before"] = []
        errors = validate(document)
        self.assertIn("entries[0].position.sfen_sha256", errors)
        self.assertIn("entries[0].position.moves_history_suffix", errors)


if __name__ == "__main__":
    unittest.main()
