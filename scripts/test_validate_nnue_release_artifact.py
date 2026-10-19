#!/usr/bin/env python3
"""Contract tests for the separately distributed NNUE artifact validator."""
from __future__ import annotations

import copy
import hashlib
import importlib.util
import json
import math
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
        verdict = "PASS" if threads == 1 else "NON_REGRESSION"
        raw = write_reference(root, f"evidence/{threads}t-games.tar.zst", f"raw-{threads}".encode())
        report = {
            "schema": "sekirei.nnue-engine-gate-evidence.v1",
            "engine_commit": "2" * 40,
            "engine_sha256": "4" * 64,
            "candidate_sha256": artifact["sha256"],
            "opening_corpus_sha256": "3" * 64,
            "raw_evidence_sha256": raw["sha256"],
            "threads": threads, "byoyomi_ms": 1000, "fv_scale": 24,
            "games": 40, "wins": 28, "draws": 0, "losses": 12,
            "verdict": verdict, "status": "complete",
            "artifact_write_failures": [], "invalid_games": [],
            "baseline_evaluator": "material", "candidate_load_acknowledged": True,
            "baseline_weights_loaded": False,
            "sprt": {
                "method": "paired-trinomial", "elo0": 0 if threads == 1 else -10,
                "elo1": 20 if threads == 1 else 0, "alpha": 0.05, "beta": 0.05,
                "require_complete_pairs": True, "analyzed_pairs": 20,
                "llr": 3.0, "lower_bound": -math.log(19), "upper_bound": math.log(19),
            },
        }
        condition = {
            "threads": threads, "verdict": verdict, "byoyomi_ms": 1000, "games": 40,
            "evidence": write_reference(root, f"evidence/{threads}t-result.json", json.dumps(report).encode()),
            "raw_evidence": raw,
        }
        if threads == 4:
            condition["non_regression_margin_elo"] = 10
        conditions.append(condition)
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
            "engine_sha256": "4" * 64,
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

    def test_v2_accepts_distinct_frozen_corpora_per_thread_condition(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            document = v2_card(root)
            gate = document["current_engine_gate"]
            del gate["opening_corpus_sha256"]
            for condition, corpus_sha256 in zip(
                gate["conditions"], ("3" * 64, "5" * 64), strict=True
            ):
                condition["opening_corpus_sha256"] = corpus_sha256
                reference = condition["evidence"]
                report = json.loads((root / reference["path"]).read_text())
                report["opening_corpus_sha256"] = corpus_sha256
                condition["evidence"] = write_reference(
                    root, reference["path"], json.dumps(report).encode()
                )
            self.assertEqual(ARTIFACT.validate(document, root, "2" * 40), [])

    def test_v2_rejects_missing_common_and_condition_corpus_identities(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            document = v2_card(root)
            del document["current_engine_gate"]["opening_corpus_sha256"]
            errors = ARTIFACT.validate(document, root, "2" * 40)
            self.assertIn("current_engine_gate.opening_corpus_sha256", errors)
            self.assertIn(
                "current_engine_gate.conditions[0].opening_corpus_sha256", errors
            )

    def test_v2_rejects_non_string_identity_and_version_metadata(self) -> None:
        cases = [
            (("artifact", "sha256"), "artifact"),
            (("training", "engine_commit"), "training.engine_commit"),
            (("training", "dataset_manifest", "sha256"), "training.dataset_manifest"),
            (("training", "whole_game_holdout_manifest", "sha256"), "training.whole_game_holdout_manifest"),
            (("current_engine_gate", "engine_commit"), "current_engine_gate.engine_commit"),
            (("current_engine_gate", "engine_sha256"), "current_engine_gate.engine_sha256"),
            (("current_engine_gate", "opening_corpus_sha256"), "current_engine_gate.opening_corpus_sha256"),
            (("distribution", "release_version"), "distribution"),
            (("current_engine_gate", "conditions", 0, "evidence", "sha256"), "current_engine_gate.conditions[0].evidence"),
            (("current_engine_gate", "conditions", 0, "raw_evidence", "sha256"), "current_engine_gate.conditions[0].raw_evidence"),
        ]
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            valid = v2_card(root)
            for path, expected in cases:
                for value in (None, True, 42, ["bad"], {"bad": "value"}):
                    with self.subTest(path=path, value=value):
                        document = copy.deepcopy(valid)
                        target = document
                        for key in path[:-1]:
                            target = target[key]
                        target[path[-1]] = value
                        self.assertIn(expected, ARTIFACT.validate(document, root, "2" * 40))

    def test_v1_rejects_non_string_hash_metadata(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            payload = b"SEKIRW01fixture"
            write_reference(root, "weights/model.bin", payload)
            for value in (None, True, 42, ["bad"], {"bad": "value"}):
                for baseline in (False, True):
                    with self.subTest(value=value, baseline=baseline):
                        document = v1_card(payload)
                        if baseline:
                            document["strength_gate"]["baseline"]["sha256"] = value
                        else:
                            document["sha256"] = value
                        expected = "strength_gate.baseline" if baseline else "sha256.format"
                        self.assertIn(expected, ARTIFACT.validate(document, root))

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

    def test_v2_rejects_hash_correct_evidence_with_wrong_identity_or_result(self) -> None:
        cases = {
            "schema": "unrelated.summary.v1", "engine_commit": "9" * 40,
            "engine_sha256": "9" * 64,
            "candidate_sha256": "9" * 64, "opening_corpus_sha256": "9" * 64,
            "raw_evidence_sha256": "9" * 64, "threads": 4, "games": 42,
            "byoyomi_ms": 500, "fv_scale": 16, "verdict": "INCONCLUSIVE",
            "status": "partial", "invalid_games": ["timeout"],
            "artifact_write_failures": ["missing record"], "baseline_evaluator": "nnue",
            "candidate_load_acknowledged": False, "baseline_weights_loaded": True,
        }
        for key, value in cases.items():
            with self.subTest(key=key), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                card = v2_card(root)
                condition = card["current_engine_gate"]["conditions"][0]
                reference = condition["evidence"]
                report = json.loads((root / reference["path"]).read_text())
                report[key] = value
                condition["evidence"] = write_reference(root, reference["path"], json.dumps(report).encode())
                self.assertTrue(ARTIFACT.validate(card, root, "2" * 40), key)

    def test_v2_rejects_inconsistent_counts_or_unaccepted_sprt(self) -> None:
        cases = [
            ("wins", 29), ("wins", True), ("threads", True),
            ("sprt.llr", 2.638), ("sprt.llr", float("nan")),
            ("sprt.upper_bound", 2.5), ("sprt.alpha", 0.10),
            ("sprt.elo0", -10), ("sprt.method", "unpaired"),
            ("sprt.require_complete_pairs", False), ("sprt.analyzed_pairs", 19),
        ]
        for key, value in cases:
            with self.subTest(key=key), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                card = v2_card(root)
                condition = card["current_engine_gate"]["conditions"][0]
                reference = condition["evidence"]
                report = json.loads((root / reference["path"]).read_text())
                if key.startswith("sprt."):
                    report["sprt"][key.split(".")[1]] = value
                else:
                    report[key] = value
                condition["evidence"] = write_reference(root, reference["path"], json.dumps(report).encode())
                self.assertTrue(ARTIFACT.validate(card, root, "2" * 40), key)

    def test_v2_requires_explicit_four_thread_non_regression_margin(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            card = v2_card(root)
            del card["current_engine_gate"]["conditions"][1]["non_regression_margin_elo"]
            self.assertTrue(ARTIFACT.validate(card, root, "2" * 40))

    def test_v2_accepts_four_thread_superiority_with_matching_sprt(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            card = v2_card(root)
            condition = card["current_engine_gate"]["conditions"][1]
            condition["verdict"] = "PASS"
            del condition["non_regression_margin_elo"]
            reference = condition["evidence"]
            report = json.loads((root / reference["path"]).read_text())
            report["verdict"] = "PASS"
            report["sprt"].update(elo0=0, elo1=20)
            condition["evidence"] = write_reference(root, reference["path"], json.dumps(report).encode())
            self.assertEqual(ARTIFACT.validate(card, root, "2" * 40), [])

    def test_v2_rejects_malformed_duplicate_and_oversized_gate_json(self) -> None:
        payloads = [b"{", b"[]", b'{"schema":"a","schema":"b"}', b" " * (1024 * 1024 + 1)]
        for payload in payloads:
            with self.subTest(size=len(payload)), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                card = v2_card(root)
                condition = card["current_engine_gate"]["conditions"][0]
                relative = condition["evidence"]["path"]
                condition["evidence"] = write_reference(root, relative, payload)
                self.assertTrue(ARTIFACT.validate(card, root, "2" * 40))

    def test_v2_rejects_invalid_condition_time_or_incomplete_pair_count(self) -> None:
        for key, value in [("byoyomi_ms", 0), ("byoyomi_ms", True), ("games", 39), ("threads", True)]:
            with self.subTest(key=key, value=value), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                card = v2_card(root)
                card["current_engine_gate"]["conditions"][0][key] = value
                self.assertTrue(ARTIFACT.validate(card, root, "2" * 40))

    def test_v2_gate_evidence_requires_utf8(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            card = v2_card(root)
            condition = card["current_engine_gate"]["conditions"][0]
            relative = condition["evidence"]["path"]
            text = (root / relative).read_text()
            condition["evidence"] = write_reference(root, relative, text.encode("utf-16"))
            self.assertIn("current_engine_gate.conditions[0].evidence.json",
                          ARTIFACT.validate(card, root, "2" * 40))

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
