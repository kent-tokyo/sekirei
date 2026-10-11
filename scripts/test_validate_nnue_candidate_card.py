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

    def test_passed_material_gate_is_accepted_without_authorizing_distribution(self) -> None:
        card, _ = fixture()
        card["material_gate"] = {
            "status": "PASS",
            "required_before_distribution_recommendation": False,
            "protocol": {
                "method": "paired trinomial SPRT",
                "elo0": 0,
                "elo1": 20,
                "alpha": 0.05,
                "beta": 0.05,
                "complete_colour_swapped_pairs_required": True,
            },
            "engine": {"sha256": "1" * 64},
            "candidate_sha256": card["artifact"]["sha256"],
            "opening_corpus_sha256": "2" * 64,
            "conditions": [
                {
                    "byoyomi_ms": 500,
                    "pairs": 2,
                    "games": 4,
                    "wins": 3,
                    "draws": 0,
                    "losses": 1,
                    "llr": 3.0,
                    "upper_bound": 2.94,
                    "evidence": "docs/experiments/gate/result.json",
                }
            ],
            "raw_evidence_archives": [
                {
                    "sha256": "3" * 64,
                    "repository_status": "local_only",
                }
            ],
            "scope": "Local evidence; not a default-adoption decision.",
        }
        self.assertEqual(MODULE.validate(card), [])
        self.assertFalse(card["distribution"]["bundled"])
        self.assertFalse(card["distribution"]["default_evaluator"])

    def test_material_gate_rejects_hash_and_incomplete_pair_counts(self) -> None:
        card, _ = fixture()
        card["material_gate"] = {
            "status": "PASS",
            "required_before_distribution_recommendation": False,
            "protocol": {
                "method": "paired trinomial SPRT",
                "elo0": 0,
                "elo1": 20,
                "alpha": 0.05,
                "beta": 0.05,
                "complete_colour_swapped_pairs_required": True,
            },
            "engine": {"sha256": "1" * 64},
            "candidate_sha256": "0" * 64,
            "opening_corpus_sha256": "2" * 64,
            "conditions": [
                {
                    "byoyomi_ms": 500,
                    "pairs": 2,
                    "games": 3,
                    "wins": 2,
                    "draws": 0,
                    "losses": 1,
                    "llr": 2.0,
                    "upper_bound": 2.94,
                    "evidence": "local/result.json",
                }
            ],
            "raw_evidence_archives": [],
            "scope": "Local evidence.",
        }
        errors = MODULE.validate(card)
        self.assertIn("material_gate.candidate_sha256", errors)
        self.assertIn("material_gate.conditions[0]", errors)
        self.assertIn("material_gate.raw_evidence_archives", errors)
        self.assertIn("material_gate.scope", errors)


if __name__ == "__main__":
    unittest.main()
