#!/usr/bin/env python3
"""Tests for the unpublished self-NNUE candidate-card validator."""
from __future__ import annotations

import hashlib
import importlib.util
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "candidate_card", ROOT / "validate_nnue_candidate_card.py"
)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def fixture() -> tuple[dict, bytes]:
    descriptor = b"Sekirei HalfKP 256x2-32-32 own training (10 positions, lam 1.0)"
    payload = (
        MODULE.HALFKP_VERSION.to_bytes(4, "little")
        + (0x12345678).to_bytes(4, "little")
        + len(descriptor).to_bytes(4, "little")
        + descriptor
        + b"fixture"
    )
    card = {
        "schema": "sekirei.nnue-candidate-card.v1",
        "candidate_id": "fixture",
        "artifact": {
            "filename": "fixture.bin",
            "sha256": hashlib.sha256(payload).hexdigest(),
            "bytes": len(payload),
            "format": MODULE.EXPECTED_FORMAT,
            "version_word": "0x7AF32F16",
            "architecture_descriptor": descriptor.decode(),
            "fv_scale": 24,
        },
        "distribution": {
            "status": "local_only",
            "bundled": False,
            "default_evaluator": False,
        },
        "training": {
            "external_teacher_used": False,
            "source_generations": {"fresh_only": True, "positions": 10},
            "fv_scale": 24,
        },
        "candidate_relative_evidence": {
            "status": "accepted_private_incumbent",
            "games": 4,
            "wins": 2,
            "draws": 1,
            "losses": 1,
            "scope": "Private comparison; not a material gate or rating.",
        },
        "material_gate": {"status": "NOT_RUN"},
        "evidence_gaps": ["raw_match_records"],
        "derived_estimates": {"directly_measured": False},
    }
    return card, payload


class CandidateCardTests(unittest.TestCase):
    def test_valid_card_and_artifact(self) -> None:
        card, payload = fixture()
        self.assertEqual(MODULE.validate(card), [])
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "fixture.bin"
            path.write_bytes(payload)
            self.assertEqual(MODULE.validate_artifact(card, path), [])

    def test_distribution_cannot_claim_default(self) -> None:
        card, _ = fixture()
        card["distribution"]["default_evaluator"] = True
        self.assertIn("distribution.boundary", MODULE.validate(card))

    def test_artifact_hash_mismatch_is_rejected(self) -> None:
        card, payload = fixture()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "fixture.bin"
            path.write_bytes(payload + b"changed")
            errors = MODULE.validate_artifact(card, path)
        self.assertIn("artifact.bytes", errors)
        self.assertIn("artifact.sha256", errors)


if __name__ == "__main__":
    unittest.main()
