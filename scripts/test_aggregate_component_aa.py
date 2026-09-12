import tempfile
import unittest
from pathlib import Path
import json

from aggregate_component_aa import aggregate, evaluate_aa_window
from run_component_benchmark import EXPECTED_CASES


class AggregateComponentAaTests(unittest.TestCase):
    def test_aggregates_adjacent_pairs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for index, value in enumerate((10.0, 5.0, 8.0, 8.0), 1):
                path = root / f"aa{index}"
                path.mkdir()
                (path / "provenance.json").write_text(json.dumps({
                    "schema": "sekirei.component-capture.v1",
                    "head": "head",
                    "binary_sha256": "binary",
                    "nnue": "mode",
                    "dirty_status": "",
                }))
                (path / "validated_summary.json").write_text(json.dumps({
                    f"{operation}/{library}": {
                        "p50_ns": value,
                        "p95_ns": value,
                        "units": 1,
                    }
                    for operation, library in EXPECTED_CASES
                }))
            result = aggregate([root / f"aa{index}" for index in range(1, 5)])
            self.assertEqual(result["pair_count"], 2)
            self.assertEqual(result["case_count"], len(EXPECTED_CASES))
            self.assertAlmostEqual(result["pair_rows"][0]["geomean"], 2.0)
            self.assertAlmostEqual(result["overall_geomean"], 1.41421356237)
            self.assertLess(result["pair_geomean_ci95"]["low"], 1.5)
            self.assertGreater(result["pair_geomean_ci95"]["high"], 1.5)

    def test_rejects_odd_capture_count(self):
        with self.assertRaises(ValueError):
            aggregate([Path("aa1")])

    def test_rejects_mixed_binary_provenance(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            paths = []
            for index in (1, 2):
                path = root / f"aa{index}"
                path.mkdir()
                (path / "provenance.json").write_text(json.dumps({
                    "schema": "sekirei.component-capture.v1",
                    "head": "head",
                    "binary_sha256": f"binary-{index}",
                    "nnue": "mode",
                    "dirty_status": "",
                }))
                (path / "validated_summary.json").write_text(json.dumps({
                    f"{operation}/{library}": {"p50_ns": 1.0, "p95_ns": 1.0, "units": 1}
                    for operation, library in EXPECTED_CASES
                }))
                paths.append(path)
            with self.assertRaises(ValueError):
                aggregate(paths)

    def test_marks_noise_outside_gate_window_inconclusive(self):
        result = {
            "pair_geomean_ci95": {"low": 0.97, "high": 1.01},
        }
        self.assertEqual(evaluate_aa_window(result), "INCONCLUSIVE")

    def test_accepts_interval_inside_gate_window(self):
        result = {
            "pair_geomean_ci95": {"low": 0.99, "high": 1.01},
        }
        self.assertEqual(evaluate_aa_window(result), "PASS")


if __name__ == "__main__":
    unittest.main()
