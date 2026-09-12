import tempfile
import unittest
from pathlib import Path
import json

from aggregate_component_aa import aggregate
from run_component_benchmark import EXPECTED_CASES


class AggregateComponentAaTests(unittest.TestCase):
    def test_aggregates_adjacent_pairs(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for index, value in enumerate((10.0, 5.0, 8.0, 8.0), 1):
                path = root / f"aa{index}"
                path.mkdir()
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

    def test_rejects_odd_capture_count(self):
        with self.assertRaises(ValueError):
            aggregate([Path("aa1")])


if __name__ == "__main__":
    unittest.main()
