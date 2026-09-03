import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from record_resume_run import record
from validate_resume_manifest import validate
from attach_resume_manifest import attach
from validate_release_manifest import validate as validate_release


class ResumeManifestTests(unittest.TestCase):
    def test_records_checkpoint_and_execution_lineage(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            checkpoint = root / "run.resume.json"
            log = root / "run.log"
            output = root / "resume-manifest.json"
            checkpoint.write_text(json.dumps({
                "schema": "sekirei.resume-checkpoint.v1",
                "epoch_completed": 1,
                "next_game_index": 3,
                "config_fingerprint": "abc123",
                "teacher_cache": {"sfen": 42},
                "optimizer": {"schema": "sekirei.adam-checkpoint.v1", "step": 9},
            }))
            log.write_text("resumed complete state from x\nstopping after requested atomic resume checkpoint\n")
            record(checkpoint, log, output, "fixture.csa")
            manifest = json.loads(output.read_text())
            self.assertEqual(manifest["schema"], "sekirei.resume-manifest.v1")
            self.assertEqual(manifest["checkpoint"]["next_game_index"], 3)
            self.assertEqual(manifest["checkpoint"]["teacher_cache_entries"], 1)
            self.assertTrue(manifest["execution"]["resume_loaded"])
            self.assertTrue(manifest["execution"]["stopped_after_checkpoint"])
            self.assertEqual(validate(manifest), [])

    def test_attaches_resume_evidence_without_mutating_release_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            release = root / "release.json"
            resume = root / "resume.json"
            output = root / "release-with-resume.json"
            release.write_text(json.dumps({
                "schema": "sekirei.release-manifest.v1", "release": "v0.3.24",
                "commit": "a" * 40, "packages": {name: "0.3.24" for name in {
                    "sekirei", "sekirei-core", "sekirei-bench", "sekirei-csa", "sekirei-match-runner", "sekirei-train"}},
                "dependencies": {"lineprior": "0.11.1"},
                "binary": {"path": "x", "sha256": "0" * 64},
                "publish": {"registry": "crates.io", "status": "verified", "workflow_run": "1", "crates": ["sekirei", "sekirei-core", "sekirei-bench", "sekirei-csa", "sekirei-match-runner", "sekirei-train"]},
                "internal_measurement": {"spec_top_n": 0, "threads": 1, "parallel": 1, "strength_claim": False},
                "external_opponents": {"status": "not_run", "configuration": "separate"},
            }))
            resume.write_text(json.dumps({
                "schema": "sekirei.resume-manifest.v1",
                "checkpoint": {"path": "x", "sha256": "1" * 64, "schema": "sekirei.resume-checkpoint.v1", "epoch_completed": 1, "next_game_index": 2, "config_fingerprint": "fp", "optimizer_step": 3, "teacher_cache_entries": 4},
                "execution": {"dataset": "d", "log_path": "l", "log_sha256": "2" * 64, "resume_loaded": True, "stopped_after_checkpoint": True},
            }))
            original = json.loads(release.read_text())
            attached = attach(release, resume, output)
            self.assertEqual(validate_release(original), [])
            self.assertNotIn("resume_verification", original)
            self.assertEqual(attached["resume_verification"]["status"], "verified")
            self.assertEqual(attached["resume_verification"]["optimizer_step"], 3)
            self.assertEqual(validate_release(attached), [])

    def test_rejects_corrupted_lineage(self):
        self.assertIn("checkpoint.sha256", validate({"schema": "sekirei.resume-manifest.v1", "checkpoint": {"sha256": "bad"}, "execution": {}}))


if __name__ == "__main__":
    unittest.main()
