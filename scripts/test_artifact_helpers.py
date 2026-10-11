#!/usr/bin/env python3
"""Unit tests for shared script artifact primitives."""

from __future__ import annotations

import hashlib
import json
import tempfile
import unittest
from pathlib import Path

from sekirei_tools.artifacts import (
    atomic_write_json,
    canonical_json_sha256,
    file_sha256,
)


class ArtifactHelperTests(unittest.TestCase):
    def test_file_sha256_hashes_exact_bytes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "artifact.bin"
            value = b"sekirei\x00artifact\n"
            path.write_bytes(value)
            self.assertEqual(file_sha256(path), hashlib.sha256(value).hexdigest())

    def test_canonical_json_hash_ignores_mapping_order(self) -> None:
        left = {"z": [2, 1], "a": "駒"}
        right = {"a": "駒", "z": [2, 1]}
        self.assertEqual(canonical_json_sha256(left), canonical_json_sha256(right))

    def test_atomic_write_json_replaces_complete_document(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "nested" / "result.json"
            atomic_write_json(path, {"old": True})
            atomic_write_json(path, {"new": [1, 2, 3]})
            self.assertEqual(json.loads(path.read_text(encoding="utf-8")), {"new": [1, 2, 3]})
            self.assertEqual(list(path.parent.glob(f".{path.name}.*")), [])


if __name__ == "__main__":
    unittest.main()

