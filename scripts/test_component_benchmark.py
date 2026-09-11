import unittest

from run_component_benchmark import validate_samples


def fixture():
    return "schema=sekirei.component-benchmark.v1\n" + "".join(
        f"sample,update,sekirei,{sample},100,12340,123.4000,6\n"
        for sample in range(21)
    ) + "summary,update,sekirei,123.4000,123.4000,6\n"


class ComponentSamplesTest(unittest.TestCase):
    def test_complete_run(self):
        self.assertEqual(validate_samples(fixture())["update/sekirei"]["p50_ns"], 123.4)

    def test_rejects_missing_or_duplicate_sample(self):
        line = "sample,update,sekirei,0,100,12340,123.4000,6\n"
        for text in (fixture().replace(line, ""), fixture() + line):
            with self.assertRaises(ValueError):
                validate_samples(text)

    def test_rejects_invalid_counts_timing_and_percentiles(self):
        for old, new in ((",100,12340,", ",0,12340,"),
                         (",100,12340,", ",100,12341,"),
                         (",123.4000,6", ",NaN,6"),
                         ("summary,update,sekirei,123.4000,123.4000,6", "summary,update,sekirei,123.4000,123.4000,1"),
                         ("summary,update,sekirei,123.4000", "summary,update,sekirei,124.4000")):
            with self.assertRaises(ValueError):
                validate_samples(fixture().replace(old, new))


if __name__ == "__main__":
    unittest.main()
