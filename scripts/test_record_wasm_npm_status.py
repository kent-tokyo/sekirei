#!/usr/bin/env python3

import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from record_wasm_npm_status import updated_manifest
from validate_release_manifest import validate


MANIFEST = Path(__file__).parents[1] / "docs/releases/manifests/release-manifest-v0.5.0.json"


class NpmStatusTests(unittest.TestCase):
    def setUp(self):
        self.source = json.loads(MANIFEST.read_text(encoding="utf-8"))

    def test_verified_registry_publication_has_integrity(self):
        updated = updated_manifest(self.source, status="published", result="verified",
                                   workflow_run="123", integrity="sha512-" + "a" * 86 + "==")
        self.assertEqual(validate(updated), [])
        self.assertEqual(updated["webassembly"]["npm_registry_version"], "0.5.0")
        self.assertEqual(self.source["webassembly"]["npm_registry_status"], "not_published")
        self.assertFalse(any("not published to the npm registry" in note for note in updated["known_limitations"]))

    def test_failed_attempt_is_unverified_not_falsely_published(self):
        updated = updated_manifest(self.source, status="unverified", result="failed",
                                   workflow_run="123", integrity=None)
        self.assertEqual(validate(updated), [])
        self.assertNotIn("npm_registry_integrity", updated["webassembly"])
        self.assertFalse(any("not published to the npm registry" in note for note in updated["known_limitations"]))

    def test_invalid_or_unsupported_evidence_is_rejected(self):
        with self.assertRaises(ValueError):
            updated_manifest(self.source, status="published", result="verified",
                             workflow_run="123", integrity=None)
        with self.assertRaises(ValueError):
            updated_manifest(self.source, status="published", result="failed",
                             workflow_run="123", integrity="sha512-abc")
        with self.assertRaises(ValueError):
            updated_manifest(self.source, status="unverified", result="failed",
                             workflow_run="0", integrity=None)


if __name__ == "__main__":
    unittest.main()
