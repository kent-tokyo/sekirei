import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from validate_corpus_legal_benchmark import HEADER, SCHEMA, SCOPE, validate_text


def fixture() -> str:
    rows = [f"case-{index:03d},sekirei_generate_vec,10,9,12,{SCOPE}" for index in range(128)]
    rows += [f"case-{index:03d},rsshogi_generate_move32,11,10,13,{SCOPE}" for index in range(128)]
    return "\n".join([
        f"schema={SCHEMA}",
        "cases=128,iterations=100,samples=7",
        "corpus_sha256=" + "a" * 64,
        HEADER,
        *rows,
    ])


class ValidateCorpusLegalBenchmarkTest(unittest.TestCase):
    def test_valid_fixture(self):
        self.assertEqual(validate_text(fixture()), 256)

    def test_rejects_missing_library_row(self):
        text = fixture().replace("case-127,rsshogi_generate_move32,", "missing,rsshogi_generate_move32,")
        with self.assertRaises(ValueError):
            validate_text(text)

    def test_rejects_wrong_scope(self):
        with self.assertRaises(ValueError):
            validate_text(fixture().replace(SCOPE, "wrong_scope", 1))

    def test_rejects_wrong_case_count(self):
        with self.assertRaises(ValueError):
            validate_text(fixture().replace("cases=128", "cases=127"))

    def test_rejects_missing_corpus_hash(self):
        with self.assertRaises(ValueError):
            validate_text(fixture().replace("corpus_sha256=" + "a" * 64, "corpus_sha256=missing"))

    def test_rejects_invalid_timing_order(self):
        with self.assertRaises(ValueError):
            validate_text(fixture().replace(",10,9,12,", ",10,12,9,", 1))


if __name__ == "__main__":
    unittest.main()
