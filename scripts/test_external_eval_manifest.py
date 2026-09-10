#!/usr/bin/env python3
"""Unit tests for the external evaluation manifest contract."""

import importlib.util
import json
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("external_manifest", ROOT / "validate_external_eval_manifest.py")
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class ExternalEvalManifestTests(unittest.TestCase):
    def setUp(self):
        fixture = ROOT / "fixtures" / "external_eval_manifest_v1.json"
        self.manifest = json.loads(fixture.read_text(encoding="utf-8"))

    def test_declared_yaneuraou_fixture_is_valid(self):
        MODULE.validate(self.manifest)

    def test_unknown_family_is_rejected(self):
        self.manifest["format"]["family"] = "unknown"
        with self.assertRaises(MODULE.ManifestError):
            MODULE.validate(self.manifest)

    def test_non_little_endian_is_rejected(self):
        self.manifest["format"]["endianness"] = "big"
        with self.assertRaises(MODULE.ManifestError):
            MODULE.validate(self.manifest)

    def test_missing_provenance_is_rejected(self):
        del self.manifest["provenance"]["source_commit"]
        with self.assertRaises(MODULE.ManifestError):
            MODULE.validate(self.manifest)

    def test_rejected_artifact_requires_reason(self):
        self.manifest["compatibility"]["status"] = "rejected"
        with self.assertRaises(MODULE.ManifestError):
            MODULE.validate(self.manifest)
        self.manifest["compatibility"]["rejection_reason"] = "feature layout unknown"
        MODULE.validate(self.manifest)


if __name__ == "__main__":
    unittest.main()
