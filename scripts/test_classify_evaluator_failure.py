#!/usr/bin/env python3
import unittest
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from classify_evaluator_failure import classify


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


if __name__ == "__main__":
    unittest.main()
