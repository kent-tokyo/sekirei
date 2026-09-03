import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from validate_release_manifest import validate

FIXTURE = Path(__file__).parent / "fixtures" / "release_manifest_diagnostic_v1.json"

class ReleaseManifestTests(unittest.TestCase):
    def test_operational_diagnostic_fixture_is_valid(self):
        self.assertEqual(validate(json.loads(FIXTURE.read_text())), [])

    def test_current_release_manifest_is_valid(self):
        manifest = Path(__file__).parents[1] / "release-manifest-v0.3.24.json"
        self.assertEqual(validate(json.loads(manifest.read_text())), [])

    def test_rejects_schema_and_diagnostic_classification(self):
        doc = json.loads(FIXTURE.read_text())
        doc["schema"] = "wrong"
        doc["evaluator_diagnostic"]["classification"] = "claim_more"
        errors = validate(doc)
        self.assertIn("schema", errors)
        self.assertIn("evaluator_diagnostic.classification", errors)

    def test_rejects_corrupt_resume_verification_artifact(self):
        doc = json.loads(FIXTURE.read_text())
        doc["resume_verification"] = {"schema": "sekirei.resume-manifest.v1", "status": "verified"}
        errors = validate(doc)
        self.assertIn("resume_verification.checkpoint_sha256", errors)
        self.assertIn("resume_verification.log_sha256", errors)

    def test_requires_checkpoint_and_log_artifacts(self):
        doc = json.loads(FIXTURE.read_text())
        doc["resume_verification"] = {"schema": "sekirei.resume-manifest.v1", "status": "verified", "artifacts": []}
        self.assertIn("resume_verification.artifacts", validate(doc))

if __name__ == "__main__": unittest.main()
