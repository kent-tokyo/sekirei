import unittest

from compare_component_benchmarks import compare
from run_component_benchmark import EXPECTED_CASES


def summary(multiplier=1.0):
    return {
        (operation, library): 100.0 * multiplier
        for operation, library in EXPECTED_CASES
    }


class ComponentComparisonTest(unittest.TestCase):
    def test_ratio_is_baseline_over_candidate(self):
        result = compare(summary(), summary(0.5))
        self.assertEqual(result["case_count"], len(EXPECTED_CASES))
        self.assertEqual(result["overall_geomean"], 2.0)

    def test_rejects_missing_case(self):
        baseline = summary()
        candidate = summary()
        candidate.pop(next(iter(candidate)))
        with self.assertRaises(KeyError):
            compare(baseline, candidate)


if __name__ == "__main__":
    unittest.main()
