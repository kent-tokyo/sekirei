import json
import math
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from export_gate_observations import export


def manifest(candidate="c1", group="g1", feature=1.0):
    return {
        "schema": "sekirei.strength-gate-final.v1",
        "verdict": "PASS",
        "games": 40,
        "completed_colour_reversed_pairs": 20,
        "summary": {"elo_delta": 12.0, "elo_stddev": 4.0},
        "candidate": {"sha256": "a" * 64},
        "gate_observation": {
            "candidate_id": candidate,
            "group_id": group,
            "features": {"valid_cp_mse_delta": feature},
            "engine_version": "0.3.66",
            "source_commit": "deadbeef",
            "evaluator": "material",
            "time_control": {"byoyomi_ms": 500},
            "threads": 1,
            "opening_corpus": "mat1000",
            "cost": {"cpu_seconds": 20},
        },
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

    def test_different_shared_feature_sets_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            a = self.write(directory, "a.json", manifest())
            b_value = manifest("c2", "g2")
            b_value["gate_observation"]["features"] = {"other": 1.0}
            b = self.write(directory, "b.json", b_value)
            rows, report = export([a, b])
            self.assertEqual(len(rows), 1)
            self.assertEqual(report["quarantined"], 1)

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
