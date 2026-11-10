"""The future gate preflight must reject known candidates and spent roots."""
import json
from pathlib import Path
import tempfile
import unittest

from scripts.check_nnue_gate_preflight import preflight, sha


class PreflightTests(unittest.TestCase):
    def fixture(self, directory: Path):
        identity = "a" * 64
        exclusion = directory / "exclusion.json"
        exclusion.write_text(json.dumps({"identity_scheme": "scheme-v1",
                                         "complete_for_scope": True,
                                         "identities": ["b" * 64]}))
        root = directory / "roots.json"
        root.write_text(json.dumps({"status": "complete", "identity_scheme": "scheme-v1",
                                    "positions": [{"identity_sha256": identity}],
                                    "exclusions": [{"sha256": sha(exclusion)}]}))
        plan = {"candidate": "hypothesis", "baseline": "control",
                "pins": {"hypothesis": {"sha256": "c" * 64},
                         "control": {"sha256": "d" * 64},
                         "root_manifest": {"path": str(root), "sha256": sha(root)}}}
        ledger = {"schema": "sekirei.nnue-candidate-ledger.v1",
                  "entries": [{"candidate_sha256": "e" * 64,
                               "status": "H0", "evidence": [{"sha256": "f" * 64}]}]}
        closure = {"all_manifest_hashes_verified": True,
                   "manifests": [{"path": str(exclusion), "sha256": sha(exclusion)}]}
        return plan, ledger, closure, root, exclusion

    def test_qualified_fresh_candidate_passes(self):
        with tempfile.TemporaryDirectory() as temporary:
            plan, ledger, closure, _, _ = self.fixture(Path(temporary))
            self.assertEqual(preflight(plan, ledger, closure)["status"], "PASS")

    def test_known_candidate_and_missing_exclusion_fail(self):
        with tempfile.TemporaryDirectory() as temporary:
            plan, ledger, closure, root, _ = self.fixture(Path(temporary))
            plan["pins"]["hypothesis"]["sha256"] = "e" * 64
            with self.assertRaisesRegex(ValueError, "candidate already recorded"):
                preflight(plan, ledger, closure)
            plan["pins"]["hypothesis"]["sha256"] = "c" * 64
            document = json.loads(root.read_text())
            document["exclusions"] = []
            root.write_text(json.dumps(document))
            plan["pins"]["root_manifest"]["sha256"] = sha(root)
            with self.assertRaisesRegex(ValueError, "omit known exclusion"):
                preflight(plan, ledger, closure)

    def test_spent_root_fails_even_when_manifest_lists_exclusion(self):
        with tempfile.TemporaryDirectory() as temporary:
            plan, ledger, closure, _, exclusion = self.fixture(Path(temporary))
            document = json.loads(exclusion.read_text())
            document["identities"] = ["a" * 64]
            exclusion.write_text(json.dumps(document))
            closure["manifests"][0]["sha256"] = sha(exclusion)
            root = Path(plan["pins"]["root_manifest"]["path"])
            document = json.loads(root.read_text())
            document["exclusions"] = [{"sha256": sha(exclusion)}]
            root.write_text(json.dumps(document))
            plan["pins"]["root_manifest"]["sha256"] = sha(root)
            with self.assertRaisesRegex(ValueError, "overlap historical"):
                preflight(plan, ledger, closure)


if __name__ == "__main__":
    unittest.main()
