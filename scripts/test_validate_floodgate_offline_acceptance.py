#!/usr/bin/env python3
"""Tests for the offline-only Floodgate acceptance manifest validator."""
from __future__ import annotations

import json
import unittest
from pathlib import Path

from validate_floodgate_offline_acceptance import validate


ROOT = Path(__file__).resolve().parent.parent
MANIFEST = ROOT / "results/floodgate/20260912-review/fg2-offline-acceptance-manifest.json"


class OfflineAcceptanceManifestTests(unittest.TestCase):
    def test_checked_in_manifest_is_valid(self) -> None:
        self.assertEqual(validate(json.loads(MANIFEST.read_text(encoding="utf-8"))), [])

    def test_strength_claim_is_rejected(self) -> None:
        document = json.loads(MANIFEST.read_text(encoding="utf-8"))
        document["claims"]["strength"] = "established"
        self.assertIn("strength claim", validate(document))

    def test_missing_case_is_rejected(self) -> None:
        document = json.loads(MANIFEST.read_text(encoding="utf-8"))
        document["cases"] = document["cases"][:-1]
        self.assertIn("required cases", validate(document))


if __name__ == "__main__":
    unittest.main()
