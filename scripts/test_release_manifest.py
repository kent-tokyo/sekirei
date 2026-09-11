import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from validate_release_manifest import validate
from record_mcts_manifest import record
from record_mcts_transcript import parse_transcript
from verify_mcts_diagnostic import verify
from record_mcts_comparison import parse_comparison
from summarize_mcts_comparison import summarize
from aggregate_mcts_summaries import aggregate
from record_search_diagnostic import record as record_search_diagnostic
from attach_candidate_readiness import attach as attach_candidate_readiness
from attach_resume_manifest import attach as attach_resume_manifest

FIXTURE = Path(__file__).parent / "fixtures" / "release_manifest_diagnostic_v1.json"
RELEASE_MANIFEST = Path(__file__).parents[1] / "release-manifest-v0.3.29.json"

class ReleaseManifestTests(unittest.TestCase):
    def test_operational_diagnostic_fixture_is_valid(self):
        self.assertEqual(validate(json.loads(FIXTURE.read_text())), [])

    def test_current_release_manifest_is_valid(self):
        manifest = RELEASE_MANIFEST
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

    def test_rejects_resume_artifact_metadata_mismatch(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            release = root / "release.json"
            resume = root / "resume.json"
            output = root / "combined.json"
            release.write_text(RELEASE_MANIFEST.read_text(), encoding="utf-8")
            resume.write_text(json.dumps({
                "schema": "sekirei.resume-manifest.v1",
                "checkpoint": {"path": "checkpoint.json", "sha256": "a" * 64, "schema": "sekirei.resume-checkpoint.v1", "epoch_completed": 1, "next_game_index": 0, "config_fingerprint": "fp", "optimizer_step": 2, "teacher_cache_entries": 0},
                "execution": {"dataset": "d", "log_path": "run.log", "log_sha256": "b" * 64, "resume_loaded": True, "stopped_after_checkpoint": False},
            }), encoding="utf-8")
            combined = attach_resume_manifest(release, resume, output)
            combined["resume_verification"]["artifacts"][0]["sha256"] = "c" * 64
            errors = validate(combined)
            self.assertIn("resume_verification.artifacts.checkpoint_sha256_mismatch", errors)

    def test_rejects_mcts_diagnostic_strength_claim(self):
        doc = json.loads(FIXTURE.read_text())
        doc["mcts_diagnostic"]["strength_claim"] = True
        self.assertIn("mcts_diagnostic.strength_claim", validate(doc))

    def test_records_search_diagnostic_without_mutating_source(self):
        import tempfile

        source = RELEASE_MANIFEST
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "candidate.json"
            record_search_diagnostic(
                source,
                output,
                "nodes 3000",
                5,
                3000,
                tt_probes=120,
                tt_hits=40,
                order_tt=8,
                order_killer=12,
                order_countermove=6,
                order_history=94,
            )
            self.assertNotIn("search_diagnostic", json.loads(source.read_text()))

    def test_attaches_candidate_readiness_without_mutating_source(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "release.json"
            readiness = root / "readiness.json"
            output = root / "combined.json"
            source.write_text(RELEASE_MANIFEST.read_text(), encoding="utf-8")
            readiness.write_text(json.dumps({
                "schema": "sekirei.candidate-readiness.v1",
                "candidate": "weights.bin",
                "candidate_sha256": "a" * 64,
                "candidate_hash_matches": True,
                "calibration_pinned": True,
                "strict_probe": {
                    "passed": True,
                    "strict_pass": True,
                    "reload_deterministic": True,
                    "l2_distinct_values": 128,
                    "out_distinct_values": 32,
                },
                "ready_for_strength_gate": True,
                "strength_claim": False,
            }), encoding="utf-8")
            attach_candidate_readiness(source, readiness, output)
            self.assertNotIn("candidate_readiness", json.loads(source.read_text()))
            self.assertEqual(validate(json.loads(output.read_text())), [])

    def test_rejects_ready_candidate_without_all_prerequisites(self):
        doc = json.loads(FIXTURE.read_text())
        doc["candidate_readiness"] = {
            "schema": "sekirei.candidate-readiness.v1",
            "candidate": "weights.bin",
            "candidate_sha256": "a" * 64,
            "candidate_hash_matches": False,
            "calibration_pinned": False,
            "strict_probe": {
                "passed": False,
                "strict_pass": False,
                "reload_deterministic": True,
                "l2_distinct_values": 1,
                "out_distinct_values": 1,
            },
            "ready_for_strength_gate": True,
            "strength_claim": False,
        }
        errors = validate(doc)
        self.assertIn("candidate_readiness.ready_requires_candidate_hash", errors)
        self.assertIn("candidate_readiness.ready_requires_calibration", errors)
        self.assertIn("candidate_readiness.ready_requires_strict_probe", errors)

    def test_candidate_readiness_fixture_is_valid(self):
        fixture = Path(__file__).parent / "fixtures" / "candidate_readiness_v1.json"
        readiness = json.loads(fixture.read_text(encoding="utf-8"))
        self.assertEqual(readiness["schema"], "sekirei.candidate-readiness.v1")
        self.assertTrue(readiness["strict_probe"]["passed"])

    def test_rejects_passed_readiness_without_layer_statistics(self):
        document = json.loads(FIXTURE.read_text())
        document["candidate_readiness"] = json.loads(
            (Path(__file__).parent / "fixtures" / "candidate_readiness_v1.json").read_text()
        )
        document["candidate_readiness"]["strict_probe"].pop("out_distinct_values")
        errors = validate(document)
        self.assertIn("candidate_readiness.strict_probe.out_distinct_values", errors)

    def test_records_mcts_diagnostic_without_mutating_source(self):
        import tempfile

        source = RELEASE_MANIFEST
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

    def test_verifies_transcript_and_manifest_counts(self):
        import tempfile

        source = RELEASE_MANIFEST
        transcript = Path(__file__).parent / "fixtures" / "usi_smoke_shared_mcts_v0.3.29.txt"
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "candidate.json"
            record(source, output, "SharedMcts", 4, 31, 0)
            verify(output, transcript)

    def test_rejects_mismatched_transcript_and_manifest_counts(self):
        import tempfile

        source = RELEASE_MANIFEST
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "candidate.json"
            transcript = Path(directory) / "transcript.txt"
            record(source, output, "SharedMcts", 4, 31, 0)
            transcript.write_text(
                "info string shared_mcts simulations 8 arena_nodes 31 transposition_hits 0\n"
            )
            with self.assertRaisesRegex(ValueError, "simulations"):
                verify(output, transcript)

    def test_parses_deterministic_mcts_comparison(self):
        text = "\n".join(
            [
                "repeat=1 position=startpos mode=TreeMcts simulations=8 max_depth=4 nodes=10 score=0 best_move=Some(A) value_cache_hits=0",
                "repeat=1 position=startpos mode=SharedTreeMcts simulations=8 max_depth=4 nodes=9 score=0 best_move=Some(A) transposition_hits=2",
                "repeat=2 position=startpos mode=TreeMcts simulations=8 max_depth=4 nodes=10 score=0 best_move=Some(A) value_cache_hits=0",
                "repeat=2 position=startpos mode=SharedTreeMcts simulations=8 max_depth=4 nodes=9 score=0 best_move=Some(A) transposition_hits=2",
            ]
        )
        comparison = parse_comparison(text)
        self.assertEqual(comparison["repeats"], 2)
        self.assertEqual(comparison["positions"][0]["shared"]["transposition_hits"], 2)

    def test_rejects_non_deterministic_mcts_comparison(self):
        text = "\n".join(
            [
                "repeat=1 position=startpos mode=TreeMcts simulations=8 max_depth=4 nodes=10 score=0 best_move=Some(A) value_cache_hits=0",
                "repeat=1 position=startpos mode=SharedTreeMcts simulations=8 max_depth=4 nodes=9 score=0 best_move=Some(A) transposition_hits=2",
                "repeat=2 position=startpos mode=TreeMcts simulations=8 max_depth=4 nodes=11 score=0 best_move=Some(A) value_cache_hits=0",
                "repeat=2 position=startpos mode=SharedTreeMcts simulations=8 max_depth=4 nodes=9 score=0 best_move=Some(A) transposition_hits=2",
            ]
        )
        with self.assertRaisesRegex(ValueError, "non-deterministic"):
            parse_comparison(text)

    def test_summarizes_mcts_comparison_without_strength_claim(self):
        import tempfile

        source = RELEASE_MANIFEST
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "candidate.json"
            record(source, output, "SharedMcts", 4, 31, 0)
            # The single-mode diagnostic is intentionally not a comparison.
            with self.assertRaisesRegex(ValueError, "mcts_comparison"):
                summarize(output)

    def test_aggregates_comparison_summaries_by_budget(self):
        import tempfile

        source = RELEASE_MANIFEST
        with tempfile.TemporaryDirectory() as directory:
            first = Path(directory) / "first.json"
            second = Path(directory) / "second.json"
            first.write_text(
                json.dumps(
                    {
                        **json.loads(source.read_text()),
                        "mcts_comparison": {
                            "schema": "sekirei.mcts-comparison.v1",
                            "simulations": 8,
                            "max_depth": 2,
                            "repeats": 2,
                            "strength_claim": False,
                            "positions": [
                                {
                                    "name": "startpos",
                                    "tree": {"nodes": 10, "score": 0, "best_move": "A", "transposition_hits": 0},
                                    "shared": {"nodes": 9, "score": 0, "best_move": "A", "transposition_hits": 1},
                                }
                            ],
                        },
                    }
                )
            )
            second.write_text(
                json.dumps(
                    {
                        **json.loads(source.read_text()),
                        "mcts_comparison": {
                            "schema": "sekirei.mcts-comparison.v1",
                            "simulations": 64,
                            "max_depth": 4,
                            "repeats": 2,
                            "strength_claim": False,
                            "positions": [
                                {
                                    "name": "startpos",
                                    "tree": {"nodes": 100, "score": 0, "best_move": "A", "transposition_hits": 0},
                                    "shared": {"nodes": 80, "score": 0, "best_move": "A", "transposition_hits": 20},
                                }
                            ],
                        },
                    }
                )
            )
            result = aggregate([first, second])
            self.assertEqual([budget["simulations"] for budget in result["budgets"]], [8, 64])
            self.assertFalse(result["strength_claim"])

    def test_classifies_comparison_agreement(self):
        import tempfile

        source = RELEASE_MANIFEST
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "candidate.json"
            output.write_text(
                json.dumps(
                    {
                        **json.loads(source.read_text()),
                        "mcts_comparison": {
                            "schema": "sekirei.mcts-comparison.v1",
                            "simulations": 8,
                            "max_depth": 2,
                            "repeats": 2,
                            "strength_claim": False,
                            "positions": [
                                {
                                    "name": "startpos",
                                    "tree": {"nodes": 10, "score": 1, "best_move": "A", "transposition_hits": 0},
                                    "shared": {"nodes": 9, "score": 2, "best_move": "A", "transposition_hits": 1},
                                }
                            ],
                        },
                    }
                )
            )
            self.assertEqual(summarize(output)["positions"][0]["agreement"], "best_move_only")

if __name__ == "__main__": unittest.main()
