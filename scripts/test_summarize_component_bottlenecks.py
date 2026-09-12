import unittest

from summarize_component_bottlenecks import summarize


class ComponentBottleneckTest(unittest.TestCase):
    def test_ranks_sekirei_rows_and_excludes_other_libraries(self):
        result = summarize({
            "slow/sekirei": {"p50_ns": 30},
            "fast/sekirei_fixed": {"p50_ns": 10},
            "other/rsshogi": {"p50_ns": 100},
        }, hypothetical_speedup=2, top=2)
        self.assertEqual([row["operation"] for row in result["top"]], ["slow", "fast"])
        self.assertAlmostEqual(result["top"][0]["fraction_of_component_sum"], 0.75)
        self.assertAlmostEqual(result["top"][0]["amdahl_upper_bound"], 1.6)

    def test_rejects_non_acceleration(self):
        with self.assertRaises(ValueError):
            summarize({"x/sekirei": {"p50_ns": 1}}, hypothetical_speedup=1)


if __name__ == "__main__":
    unittest.main()
