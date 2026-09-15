#!/usr/bin/env python3
"""Regression tests for validate_nnue_output_metadata.py."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from validate_nnue_output_metadata import checkpoint_hash, validate


class OutputMetadataTest(unittest.TestCase):
    def write_candidate(self, directory: Path, mode: str = "residual-material") -> Path:
        weights = directory / "candidate.bin"
        weights.write_bytes(b"candidate weights")
        metadata = {
            "format": "sekirei-nnue-output-v1",
            "nnue_output": mode,
            "baseline": "material-v1" if mode == "residual-material" else None,
            "checkpoint_hash": checkpoint_hash(weights),
        }
        weights.with_suffix(".meta.json").write_text(json.dumps(metadata), encoding="utf-8")
        return weights

    def test_valid_residual_candidate(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            result = validate(self.write_candidate(Path(tmp)), "residual-material")
            self.assertEqual(result["mode"], "residual-material")

    def test_rejects_replaced_weights(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            weights = self.write_candidate(Path(tmp))
            weights.write_bytes(b"replaced bytes")
            with self.assertRaisesRegex(ValueError, "checkpoint_hash"):
                validate(weights)

    def test_rejects_missing_residual_baseline(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            weights = self.write_candidate(Path(tmp))
            metadata_path = weights.with_suffix(".meta.json")
            metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
            metadata["baseline"] = None
            metadata_path.write_text(json.dumps(metadata), encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "baseline material-v1"):
                validate(weights)


if __name__ == "__main__":
    unittest.main()
