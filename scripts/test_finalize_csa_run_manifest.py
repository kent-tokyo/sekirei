#!/usr/bin/env python3
import importlib.util
import json
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("finalize", ROOT / "scripts/finalize_csa_run_manifest.py")
finalize = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(finalize)


def test_finalizes_binary_and_weights_hashes():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        binary = root / "engine"
        weights = root / "weights.bin"
        source = root / "startup.json"
        output = root / "final.json"
        binary.write_bytes(b"engine")
        weights.write_bytes(b"weights")
        source.write_text(json.dumps({
            "schema": "sekirei.csa-run-manifest.v1",
            "binary": {"path": str(binary)},
            "weights": str(weights),
        }), encoding="utf-8")
        document = finalize.finalize(source, output, root)
        assert document["status"] == "finalized"
        assert document["binary"]["sha256"] == finalize.sha256(binary)
        assert document["weights"]["sha256"] == finalize.sha256(weights)
        assert document["hashes"]["binary"] == document["binary"]["sha256"]


def test_missing_weights_is_explicit():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        binary = root / "engine"
        source = root / "startup.json"
        output = root / "final.json"
        binary.write_bytes(b"engine")
        source.write_text(json.dumps({
            "schema": "sekirei.csa-run-manifest.v1",
            "binary": {"path": str(binary)},
            "weights": str(root / "missing.bin"),
        }), encoding="utf-8")
        document = finalize.finalize(source, output, root)
        assert document["status"] == "finalized"
        assert document["weights"]["status"] == "missing"


def test_finalizes_new_weight_artifact_shape_and_preserves_active_flag():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        binary = root / "engine"
        weights = root / "weights.bin"
        source = root / "startup.json"
        output = root / "final.json"
        binary.write_bytes(b"engine")
        weights.write_bytes(b"weights")
        source.write_text(json.dumps({
            "schema": "sekirei.csa-run-manifest.v1",
            "binary": {"path": str(binary), "bytes": 6},
            "weights": {"path": str(weights), "active": True},
        }), encoding="utf-8")
        document = finalize.finalize(source, output, root)
        assert document["status"] == "finalized"
        assert document["weights"]["active"] is True
        assert document["weights"]["sha256"] == finalize.sha256(weights)


if __name__ == "__main__":
    test_finalizes_binary_and_weights_hashes()
    test_missing_weights_is_explicit()
    test_finalizes_new_weight_artifact_shape_and_preserves_active_flag()
    print("PASS")
