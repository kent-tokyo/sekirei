import json
import math
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from export_gate_observations import canonical_sha256, export


def manifest(candidate="c1", group="g1", feature=1.0):
    declaration = {
        "schema": "sekirei.gate-observation-declaration.v1",
        "candidate_id": candidate,
        "group_id": group,
        "feature_schema_id": "sekirei.search-gate-features.v1",
        "features": {"valid_cp_mse_delta": feature},
        "group_definition": {
            "boundary": "same candidate family, evaluator, openings, and time control"
        },
        "engine_version": "0.3.68",
        "source_commit": "deadbeef",
        "evaluator": "material",
        "time_control": {"byoyomi_ms": 500},
        "threads": 1,
        "opening_corpus": "mat1000",
        "cost": {"games_limit": 40},
    }
    declaration["feature_schema_sha256"] = canonical_sha256(
        {
            "feature_schema_id": declaration["feature_schema_id"],
            "feature_names": sorted(declaration["features"]),
        }
    )
    declaration["declaration_sha256"] = canonical_sha256(declaration)
    return {
        "schema": "sekirei.strength-gate-final.v1",
        "verdict": "PASS",
        "games": 40,
        "completed_colour_reversed_pairs": 20,
        "summary": {"elo_delta": 12.0, "elo_stddev": 4.0},
        "candidate": {"sha256": "a" * 64},
        "gate_observation": declaration,
    }


class ExportTests(unittest.TestCase):
    def write(self, directory, name, value):
        path = Path(directory) / name
        path.write_text(json.dumps(value), encoding="utf-8")
        return path

    def test_valid_export_is_deterministic_and_deduplicated(self):
        with tempfile.TemporaryDirectory() as directory:
            a = self.write(directory, "a.json", manifest())
            duplicate = self.write(directory, "copy.json", manifest())
            rows, report = export([duplicate, a])
            self.assertEqual(len(rows), 1)
            self.assertEqual(report["duplicates_suppressed"], 1)
            self.assertEqual(report["readiness"], "contract_only_insufficient_groups")
            self.assertEqual(report["independent_groups"], 1)
            self.assertFalse(report["model_fitting_enabled"])
            self.assertFalse(report["calibration_enabled"])
            self.assertEqual(report["source_manifests"][0]["path"], str(a))
            self.assertEqual(len(report["source_manifests"][0]["sha256"]), 64)
            self.assertEqual(rows[0]["gate_status"], "Pass")
            self.assertIn("source_manifest_sha256", rows[0]["provenance"])

    def test_missing_group_and_shared_features_are_quarantined(self):
        with tempfile.TemporaryDirectory() as directory:
            valid = self.write(directory, "valid.json", manifest())
            missing = manifest("c2", "", 2.0)
            bad = self.write(directory, "missing.json", missing)
            rows, report = export([valid, bad])
            self.assertEqual(len(rows), 1)
            self.assertEqual(report["quarantined"], 1)
            self.assertIn("group_id", report["rejections"][0]["reason"])

    def test_non_finite_and_outcome_features_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            nonfinite = manifest()
            nonfinite["gate_observation"]["features"]["x"] = math.inf
            outcome = manifest("c2")
            outcome["gate_observation"]["features"] = {"result_elo": 1.0}
            paths = [
                self.write(directory, "nonfinite.json", nonfinite),
                self.write(directory, "outcome.json", outcome),
            ]
            rows, report = export(paths)
            self.assertEqual(rows, [])
            self.assertEqual(report["quarantined"], 2)
            self.assertEqual(report["readiness"], "blocked_no_eligible_rows")
            self.assertFalse(report["model_fitting_enabled"])
            self.assertFalse(report["acquisition_enabled"])

    def test_allow_empty_writes_deterministic_readiness_artifacts(self):
        with tempfile.TemporaryDirectory() as directory:
            source = self.write(directory, "legacy.json", {"schema": "legacy"})
            output = Path(directory) / "observations.jsonl"
            report = Path(directory) / "report.json"
            command = [
                sys.executable,
                str(Path(__file__).with_name("export_gate_observations.py")),
                str(source),
                "--output",
                str(output),
                "--report",
                str(report),
                "--allow-empty",
            ]
            completed = subprocess.run(command, check=False, capture_output=True, text=True)
            self.assertEqual(completed.returncode, 0, completed.stderr)
            self.assertEqual(output.read_text(encoding="utf-8"), "")
            document = json.loads(report.read_text(encoding="utf-8"))
            self.assertEqual(document["accepted"], 0)
            self.assertEqual(document["quarantined"], 1)
            self.assertEqual(document["readiness"], "blocked_no_eligible_rows")

    def test_different_shared_feature_sets_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            a = self.write(directory, "a.json", manifest())
            b_value = manifest("c2", "g2")
            b_value["gate_observation"]["features"] = {"other": 1.0}
            b = self.write(directory, "b.json", b_value)
            rows, report = export([a, b])
            self.assertEqual(len(rows), 1)
            self.assertEqual(report["quarantined"], 1)

    def test_terminal_result_without_pre_gate_declaration_is_quarantined(self):
        with tempfile.TemporaryDirectory() as directory:
            missing = manifest()
            missing.pop("gate_observation")
            source = self.write(directory, "missing.json", missing)
            rows, report = export([source])
            self.assertEqual(rows, [])
            self.assertEqual(report["quarantined"], 1)
            self.assertIn("pre-gate declaration", report["rejections"][0]["reason"])

    def test_conflicting_retry_for_same_candidate_group_is_quarantined(self):
        with tempfile.TemporaryDirectory() as directory:
            first = self.write(directory, "a.json", manifest())
            retry_value = manifest()
            retry_value["candidate"]["sha256"] = "b" * 64
            retry = self.write(directory, "b.json", retry_value)
            rows, report = export([first, retry])
            self.assertEqual(len(rows), 1)
            self.assertEqual(report["quarantined"], 1)
            self.assertIn("conflicting duplicate", report["rejections"][0]["reason"])


if __name__ == "__main__":
    unittest.main()
