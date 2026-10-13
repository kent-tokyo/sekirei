import importlib.util
import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock


SCRIPT = Path(__file__).with_name("validate_shogiesa_diagnostic_contract.py")
SPEC = importlib.util.spec_from_file_location("contract", SCRIPT)
contract = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(contract)


def observation(requested=4, achieved=4, perspective="side_to_move", kind="cp"):
    score = {"kind": kind, "value": 10} if kind == "cp" else {"kind": "mate", "moves": 3}
    return {
        "engine": "Sekirei",
        "engine_version": "0.3.57",
        "depth": achieved,
        "requested_depth": requested,
        "search_limit_kind": "depth",
        "score": score,
        "score_perspective": perspective,
        "score_bound": "exact",
        "bestmove": "7g7f",
    }


class DiagnosticContractTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        root = Path(self.temp.name)
        self.manifest_path = root / "manifest.json"
        self.positions_path = root / "positions.jsonl"
        self.observations_path = root / "observations.jsonl"
        self.manifest = {
            "manifest_schema_version": 1,
            "shogiesa_version": "0.11.6",
            "schema_version": 11,
            "command": "label",
            "depths": [2, 4],
            "engine_options": ["Threads=1", "UseBook=false"],
            "records_read": 1,
            "records_kept": 1,
            "requested_depth_total": 2,
            "requested_depth_underreach": 0,
            "cache_hits": 1,
            "cache_misses": 1,
            "cache_hit_rate": 0.5,
        }
        self.record = {
            "schema_version": 11,
            "observations": [observation(2, 2), observation(4, 4)],
        }
        self.verification = {
            "report_schema_version": 1,
            "shogiesa_version": "0.11.6",
            "manifest_schema_version": 1,
            "schema_version": 11,
            "command": "label",
            "depths": [2, 4],
            "engine_options": ["Threads=1", "UseBook=false"],
            "manifest_sha256": "1" * 64,
            "dataset_sha256": "2" * 64,
            "output_sha256": "3" * 64,
            "provenance": "complete",
            "missing_digests": [],
            "records_read": 1,
            "records_kept": 1,
            "cache_hits": 1,
            "cache_misses": 1,
            "cache_hit_rate": 0.5,
            "requested_depth_total": 2,
            "requested_depth_underreach": 0,
            "output": {
                "records": 1,
                "labeled_records": 1,
                "unlabeled_records": 0,
                "observations_with_candidates": 0,
                "observations_total": 2,
                "requested_depth_total": 2,
                "requested_depth_underreach": 0,
            },
        }

    def tearDown(self):
        self.temp.cleanup()

    def write(self):
        self.manifest_path.write_text(json.dumps(self.manifest), encoding="utf-8")
        self.positions_path.write_text(json.dumps({"schema_version": 11}) + "\n", encoding="utf-8")
        self.observations_path.write_text(json.dumps(self.record) + "\n", encoding="utf-8")

    def build(self, **overrides):
        self.write()
        args = {
            "version": "0.11.6",
            "verifier_executable": "typed-verifier",
            "manifest_path": self.manifest_path,
            "positions_path": self.positions_path,
            "observations_path": self.observations_path,
            "expected_depths": [2, 4],
            "label_depth": 4,
            "depth_mismatch_reason": None,
            "label_elapsed_seconds": 9,
        }
        args.update(overrides)
        with mock.patch.object(
            contract, "run_typed_verifier", return_value=self.verification
        ):
            return contract.build_contract(**args)

    def test_records_diagnostic_only_provenance_and_cache_cost(self):
        result = self.build()
        self.assertEqual(result["teacher_source"], "sekirei_internal_search")
        self.assertFalse(result["diagnostic_observations_used_as_teacher"])
        self.assertEqual(
            result["score_perspective_policy"],
            "validated_but_not_converted_diagnostic_only",
        )
        self.assertEqual(result["shogiesa_manifest_stats"]["cache_hits"], 1)
        self.assertEqual(result["shogiesa_label_elapsed_seconds"], 9)
        self.assertEqual(result["shogiesa_manifest_sha256"], "1" * 64)
        self.assertEqual(result["positions_sha256"], "2" * 64)
        self.assertEqual(result["observations_sha256"], "3" * 64)
        self.assertEqual(result["artifact_provenance"], "complete")

    def test_rejects_unsupported_version_and_schema(self):
        self.assertEqual(contract.parse_version_output("shogiesa 0.11.6"), "0.11.6")
        with self.assertRaisesRegex(contract.ContractError, "unsupported shogiesa version"):
            contract.parse_version_output("shogiesa 0.12.0")
        self.verification["schema_version"] = 12
        with self.assertRaisesRegex(contract.ContractError, "schema_version"):
            self.build()
        self.verification["schema_version"] = 11
        self.verification["shogiesa_version"] = "0.11.3"
        with self.assertRaisesRegex(contract.ContractError, "does not match"):
            self.build()

    def test_rejects_policy_mismatch_after_typed_verification(self):
        self.verification["depths"] = [4]
        with self.assertRaisesRegex(contract.ContractError, "depths"):
            self.build()
        self.verification["depths"] = [2, 4]
        self.verification["engine_options"] = ["Threads=1"]
        with self.assertRaisesRegex(contract.ContractError, "UseBook=false"):
            self.build()

    def test_typed_verifier_failure_is_fail_closed(self):
        self.write()
        completed = mock.Mock(returncode=2, stdout="", stderr="digest mismatch")
        with mock.patch.object(contract.subprocess, "run", return_value=completed):
            with self.assertRaisesRegex(contract.ContractError, "digest mismatch"):
                contract.run_typed_verifier(
                    "typed-verifier",
                    self.manifest_path,
                    self.positions_path,
                    self.observations_path,
                    False,
                )

    def test_rejects_missing_perspective_provenance(self):
        self.record["observations"][0].pop("score_perspective")
        with self.assertRaisesRegex(contract.ContractError, "missing score perspective"):
            self.build()

    def test_explicit_non_side_to_move_perspective_is_validated_but_not_imported(self):
        self.record["observations"][0]["score_perspective"] = "black"
        result = self.build()
        self.assertFalse(result["diagnostic_observations_used_as_teacher"])

    def test_rejects_non_mate_underreach(self):
        self.record["observations"][1]["depth"] = 3
        self.manifest["requested_depth_underreach"] = 1
        with self.assertRaisesRegex(contract.ContractError, "under-reached"):
            self.build()

    def test_allows_mate_underreach_but_keeps_requested_and_achieved_depth(self):
        self.record["observations"][1] = observation(4, 2, kind="mate")
        result = self.build()
        self.assertEqual(result["observation_stats"]["mate_observations"], 1)

    def test_depth_mismatch_must_be_rejected_or_explained(self):
        with self.assertRaisesRegex(contract.ContractError, "depth differs"):
            self.build(label_depth=3)
        result = self.build(label_depth=3, depth_mismatch_reason="bounded pilot")
        self.assertEqual(result["depth_mismatch_reason"], "bounded pilot")


if __name__ == "__main__":
    unittest.main()
