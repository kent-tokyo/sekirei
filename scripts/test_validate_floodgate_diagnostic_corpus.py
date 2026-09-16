#!/usr/bin/env python3
"""Tests for the diagnostic corpus validator."""
from __future__ import annotations

import copy
import hashlib
import json
import tempfile
import unittest
from pathlib import Path

from validate_floodgate_diagnostic_corpus import validate


ROOT = Path(__file__).resolve().parents[1]
CSA_FIXTURE = ROOT / "scripts/fixtures/analysis_replay_v1.csa"
ANALYSIS_FIXTURE = ROOT / "scripts/fixtures/analysis_replay_v1.analysis.jsonl"
STARTPOS_SFEN = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1"


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def fixture_entry(index: int) -> dict[str, object]:
    history = ["7g7f", "3c3d"][: index + 1]
    return {
        "entry_kind": "control",
        "label_policy": "observation_only_no_correct_move_label",
        "source": {
            "game_id": f"fixture-{index}",
            "ply": len(history),
            "csa": str(CSA_FIXTURE.relative_to(ROOT)),
            "csa_sha256": sha256_bytes(CSA_FIXTURE.read_bytes()),
            "analysis": str(ANALYSIS_FIXTURE.relative_to(ROOT)),
            "analysis_sha256": sha256_bytes(ANALYSIS_FIXTURE.read_bytes()),
        },
        "position": {
            "sfen": STARTPOS_SFEN,
            "sfen_sha256": sha256_bytes(STARTPOS_SFEN.encode("utf-8")),
            "side_to_move": "black",
            "history_before": history,
            "moves_before": history[-2:],
        },
    }


class DiagnosticCorpusTests(unittest.TestCase):
    def setUp(self) -> None:
        self.document = {
            "schema": "sekirei.floodgate-diagnostic-corpus.v1",
            "diagnostic_only": True,
            "split": "diagnostic_tuning_only",
            "strength_claim": "not_permitted",
            "entries": [fixture_entry(0), fixture_entry(1)],
        }

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
