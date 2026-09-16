#!/usr/bin/env python3
"""Tests for the offline-only Floodgate acceptance manifest validator."""
from __future__ import annotations

import unittest

from validate_floodgate_offline_acceptance import validate


def valid_manifest() -> dict[str, object]:
    case_ids = (
        "normal_game",
        "communication_loss",
        "explicit_stop",
        "record_initialization_failure",
        "supervisor_crash_and_stop",
    )
    return {
        "schema": "sekirei.floodgate-offline-acceptance.v1",
        "diagnostic_only": True,
        "cases": [
            {
                "id": case_id,
                "status": "passed",
                "fixture": f"fixture-{case_id}",
                "assertions": ["fixture contract verified"],
            }
            for case_id in case_ids
        ],
        "unverified": ["live Floodgate operation"],
        "claims": {
            "strength": "not_permitted",
            "operational_acceptance": "offline_fixture_only",
            "release_approval": "not_granted",
        },
    }


class OfflineAcceptanceManifestTests(unittest.TestCase):
    def test_self_contained_manifest_is_valid(self) -> None:
        self.assertEqual(validate(valid_manifest()), [])

    def test_strength_claim_is_rejected(self) -> None:
        document = valid_manifest()
        document["claims"]["strength"] = "established"
        self.assertIn("strength claim", validate(document))

    def test_missing_case_is_rejected(self) -> None:
        document = valid_manifest()
        document["cases"] = document["cases"][:-1]
        self.assertIn("required cases", validate(document))


if __name__ == "__main__":
    unittest.main()
