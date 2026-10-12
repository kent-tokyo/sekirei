#!/usr/bin/env python3
"""Contract tests for the separately distributed NNUE artifact validator."""
from __future__ import annotations

import hashlib
import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("artifact", ROOT / "validate_nnue_release_artifact.py")
assert SPEC and SPEC.loader
ARTIFACT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(ARTIFACT)


def v1_card(payload: bytes) -> dict:
    return {
        "schema": "sekirei.nnue-model-card.v1", "artifact": "weights/model.bin",
        "sha256": hashlib.sha256(payload).hexdigest(), "bytes": len(payload),
        "architecture": "A-flat-ps", "nnue_output": "absolute", "license": "CC-BY-4.0",
        "attribution": "Sekirei project, Kentaro Tanabe", "training": {"teacher": {}},
        "current_status": {
            "availability": "published_optional_checkpoint",
            "recommendation": "hold_after_current_material_comparison",
            "reason": "Current material comparison is complete.",
            "comparison": {
                "status": "complete", "pooled": False,
                "settings": [{"byoyomi_ms": 1000}, {"byoyomi_ms": 5000}],
            },
        },
        "strength_gate": {
            "verdict": "PASS", "games": 94,
            "scope": "Local only; not a Floodgate rating.",
            "baseline": {"path": "baseline.bin", "sha256": "0" * 64},
        },
    }


def write_reference(root: Path, relative: str, payload: bytes) -> dict:
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(payload)
    return {"path": relative, "sha256": hashlib.sha256(payload).hexdigest()}


def v2_card(root: Path) -> dict:
    descriptor = b"Sekirei HalfKP 256x2-32-32 own training (fixture)"
    payload = (
        ARTIFACT.HALFKP_VERSION.to_bytes(4, "little")
        + (0x12345678).to_bytes(4, "little")
        + len(descriptor).to_bytes(4, "little")
        + descriptor
        + b"weights"
    )
    artifact = write_reference(root, "weights/model.bin", payload)
    artifact.update(
        {
            "bytes": len(payload),
            "format": ARTIFACT.HALFKP_FORMAT,
            "version_word": "0x7AF32F16",
            "architecture_descriptor": descriptor.decode(),
            "fv_scale": 24,
            "nnue_output": "absolute",
        }
    )
    conditions = []
    for threads in (1, 4):
        conditions.append(
            {
                "threads": threads,
                "verdict": "PASS" if threads == 1 else "NON_REGRESSION",
                "games": 40,
                "evidence": write_reference(
                    root, f"evidence/{threads}t-result.json", json.dumps({"threads": threads}).encode()
                ),
                "raw_evidence": write_reference(
                    root, f"evidence/{threads}t-games.tar.zst", f"raw-{threads}".encode()
                ),
            }
        )
    return {
        "schema": "sekirei.nnue-model-card.v2",
        "model_id": "fixture-halfkp",
        "artifact": artifact,
        "license": {"spdx": "CC-BY-4.0", "attribution": "Sekirei project"},
        "training": {
            "external_teacher_used": False,
            "engine_commit": "1" * 40,
            "dataset_manifest": write_reference(root, "training/shards.json", b"shards"),
            "whole_game_holdout_manifest": write_reference(
                root, "training/holdout.json", b"holdout"
            ),
            "whole_game_split": True,
        },
        "current_engine_gate": {
            "engine_commit": "2" * 40,
            "candidate_sha256": artifact["sha256"],
            "opening_corpus_sha256": "3" * 64,
            "conditions": conditions,
        },
        "distribution": {
            "availability": "github_release_asset",
            "default_evaluator": True,
            "release_version": "0.4.0",
        },
        "evidence_gaps": [],
    }


class ReleaseArtifactTests(unittest.TestCase):
    def test_v1_historical_contract_remains_valid(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / "weights"
            target.mkdir()
            payload = b"SEKIRW01" + b"fixture"
            (target / "model.bin").write_bytes(payload)
            document = v1_card(payload)
            self.assertEqual(ARTIFACT.validate(document, root), [])
            document["bytes"] += 1
            self.assertIn("bytes", ARTIFACT.validate(document, root))

    def test_v2_halfkp_release_contract_is_valid(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            document = v2_card(root)
            self.assertEqual(ARTIFACT.validate(document, root, "2" * 40), [])

    def test_v2_requires_current_one_and_four_thread_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            document = v2_card(root)
            document["current_engine_gate"]["conditions"].pop()
            errors = ARTIFACT.validate(document, root, "2" * 40)
            self.assertIn("current_engine_gate.thread_coverage", errors)

    def test_v2_requires_explicit_expected_engine_commit(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            document = v2_card(root)
            errors = ARTIFACT.validate(document, root)
            self.assertIn("current_engine_gate.expected_commit_required", errors)

    def test_v2_rejects_stale_engine_and_tampered_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            document = v2_card(root)
            evidence = root / document["current_engine_gate"]["conditions"][0]["evidence"]["path"]
            evidence.write_text("tampered", encoding="utf-8")
            errors = ARTIFACT.validate(document, root, "4" * 40)
            self.assertIn("current_engine_gate.engine_commit.expected", errors)
            self.assertIn("current_engine_gate.conditions[0].evidence.sha256", errors)

    def test_v2_requires_complete_provenance_and_no_gaps(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            document = v2_card(root)
            document["training"]["whole_game_split"] = False
            document["evidence_gaps"] = ["raw_match_records"]
            errors = ARTIFACT.validate(document, root)
            self.assertIn("training.whole_game_split", errors)
            self.assertIn("evidence_gaps", errors)

    def test_v2_rejects_reference_outside_bundle_root(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            document = v2_card(root)
            document["training"]["dataset_manifest"] = {
                "path": "../outside.json",
                "sha256": "0" * 64,
            }
            errors = ARTIFACT.validate(document, root)
            self.assertIn("training.dataset_manifest.outside_root", errors)


if __name__ == "__main__":
    unittest.main()
