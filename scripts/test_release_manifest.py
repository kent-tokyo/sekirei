import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from validate_release_manifest import validate
from record_mcts_manifest import record
from record_mcts_transcript import parse_transcript

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

    def test_rejects_mcts_diagnostic_strength_claim(self):
        doc = json.loads(FIXTURE.read_text())
        doc["mcts_diagnostic"]["strength_claim"] = True
        self.assertIn("mcts_diagnostic.strength_claim", validate(doc))

    def test_records_mcts_diagnostic_without_mutating_source(self):
        import tempfile

        source = Path(__file__).parents[1] / "release-manifest-v0.3.28.json"
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "candidate.json"
            record(source, output, "SharedMcts", 4, 31, 0)
            self.assertNotIn("mcts_diagnostic", json.loads(source.read_text()))
            self.assertEqual(validate(json.loads(output.read_text())), [])

    def test_parses_latest_shared_mcts_transcript_line(self):
        counts = parse_transcript(
            "info depth 2 score cp 0\n"
            "info string shared_mcts simulations 4 arena_nodes 31 transposition_hits 0\n"
            "info string shared_mcts simulations 8 arena_nodes 47 transposition_hits 3\n"
        )
        self.assertEqual(counts, {"simulations": 8, "arena_nodes": 47, "transposition_hits": 3})

    def test_rejects_transcript_without_shared_mcts_line(self):
        with self.assertRaises(ValueError):
            parse_transcript("info depth 2 score cp 0\n")

if __name__ == "__main__": unittest.main()
