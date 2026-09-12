import json
import unittest
from pathlib import Path

from validate_sequence_contract import DEFAULT_PATH, validate


class SequenceContractTest(unittest.TestCase):
    def test_checked_in_contract(self):
        self.assertEqual(validate(DEFAULT_PATH), 16)

    def test_rejects_missing_series(self):
        document = json.loads(DEFAULT_PATH.read_text(encoding="utf-8"))
        document["series"] = document["series"][:-1]
        with self.assertRaises(ValueError):
            class FakePath:
                def read_text(self, encoding):
                    return json.dumps(document)

            validate(FakePath())


if __name__ == "__main__":
    unittest.main()
