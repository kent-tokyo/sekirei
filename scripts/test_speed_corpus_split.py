import json
import tempfile
import unittest
from pathlib import Path

from validate_speed_corpus_split import CORPUS, SPLIT, validate


class SpeedCorpusSplitTest(unittest.TestCase):
    def test_checked_in_split_is_valid(self):
        validate()

    def test_rejects_source_hash_drift(self):
        split = json.loads(SPLIT.read_text(encoding="utf-8"))
        split["source_corpus_sha256"] = "0" * 64
        with self.assertRaises(ValueError):
            with tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / "split.json"
                path.write_text(json.dumps(split), encoding="utf-8")
                validate(CORPUS, path)


if __name__ == "__main__":
    unittest.main()
