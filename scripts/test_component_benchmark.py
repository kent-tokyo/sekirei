import unittest

from run_component_benchmark import EXPECTED_CASES, validate_samples


def fixture():
    rows = [
        "schema=sekirei.component-benchmark.v1\n",
        "samples=21;target_sample_ms=50;minimum_sample_ms=20;weights=default_lcg;global_initialization=excluded;debug_assertions=false\n",
    ]
    for operation, library in sorted(EXPECTED_CASES):
        rows.extend(
            f"sample,{operation},{library},{sample},100,50000000,500000.0000,1\n"
            for sample in range(21)
        )
        rows.append(f"summary,{operation},{library},500000.0000,500000.0000,1\n")
    return "".join(rows)


class ComponentSamplesTest(unittest.TestCase):
    def test_complete_run(self):
        result = validate_samples(fixture())
        self.assertEqual(len(result), len(EXPECTED_CASES))
        self.assertEqual(result["startpos/sekirei_generate_vec"]["p50_ns"], 500000.0)

    def test_rejects_missing_or_duplicate_sample(self):
        line = "sample,startpos,sekirei_generate_vec,0,100,50000000,500000.0000,1\n"
        for text in (fixture().replace(line, ""), fixture() + line):
            with self.assertRaises(ValueError):
                validate_samples(text)

    def test_rejects_invalid_counts_timing_and_percentiles(self):
        for old, new in ((",100,50000000,", ",0,50000000,"),
                         (",100,50000000,", ",100,50000001,"),
                         (",500000.0000,1", ",NaN,1"),
                         ("summary,startpos,sekirei_generate_vec,500000.0000,500000.0000,1", "summary,startpos,sekirei_generate_vec,500000.0000,500000.0000,2"),
                         ("summary,startpos,sekirei_generate_vec,500000.0000,500000.0000,1", "summary,startpos,sekirei_generate_vec,500001.0000,500000.0000,1"),
                         (",100,50000000,", ",100,19999999,")):
            with self.assertRaises(ValueError):
                validate_samples(fixture().replace(old, new))

    def test_rejects_missing_required_case(self):
        text = fixture().replace(
            "summary,encode_raw_list,rsshogi,500000.0000,500000.0000,1\n", ""
        )
        with self.assertRaises(ValueError):
            validate_samples(text)


if __name__ == "__main__":
    unittest.main()
