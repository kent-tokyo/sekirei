#!/usr/bin/env python3
import unittest
import sys
import json
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from classify_evaluator_failure import attach_to_release_manifest, classify, make_record


class EvaluatorFailureClassificationTests(unittest.TestCase):
    def test_constant_output_is_model_quality(self):
        result = classify({"probe": {"constant_output": True}})
        self.assertEqual(result["classification"], "model_quality")

    def test_reload_mismatch_is_inference(self):
        result = classify({"probe": {"reload_deterministic": False}})
        self.assertEqual(result["classification"], "inference_scale_or_quantization")

    def test_search_timeout_is_search_budget(self):
        result = classify({"probe": {"strict_pass": True}, "search": {"timeout_rate": 0.1}})
        self.assertEqual(result["classification"], "search_cost_budget")

    def test_missing_evidence_stays_undetermined(self):
        result = classify({"probe": {"strict_pass": True}})
        self.assertEqual(result["classification"], "undetermined")
        self.assertEqual(result["confidence"], "low")

    def test_separate_evidence_files_are_combined_without_inference(self):
        with tempfile.TemporaryDirectory() as directory:
            directory = Path(directory)
            probe = directory / "probe.json"
            gate = directory / "gate.json"
            probe.write_text(json.dumps({"strict_pass": True}), encoding="utf-8")
            gate.write_text(json.dumps({"games": 12, "elo_diff": 4}), encoding="utf-8")
            record = make_record({"probe": probe, "gate": gate})
        self.assertEqual(record["gate"]["games"], 12)
        self.assertEqual(classify(record)["classification"], "undetermined")

    def test_report_can_be_attached_to_release_manifest(self):
        manifest = {"schema": "sekirei.release-manifest.v1", "release": "v0.3.24"}
        report = {"classification": "undetermined", "confidence": "low", "reasons": []}
        attached = attach_to_release_manifest(manifest, report)
        self.assertEqual(attached["release"], "v0.3.24")
        self.assertEqual(attached["evaluator_diagnostic"]["schema"], "sekirei.evaluator-diagnostic.v1")


if __name__ == "__main__":
    unittest.main()
