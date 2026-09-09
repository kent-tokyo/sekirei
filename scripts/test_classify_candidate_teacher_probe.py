import importlib.util
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "classify_candidate_teacher_probe", ROOT / "classify_candidate_teacher_probe.py"
)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class ClassificationTests(unittest.TestCase):
    def test_multi_digit_hand_count(self):
        self.assertEqual(MODULE.hand_units("R2BG3SNL5Pn2l10p"), 27)

    def test_feature_bands(self):
        row = {"sfen": "4k4/9/9/9/9/9/9/9/4K4 b R2p 121"}
        result = MODULE.features(row)
        self.assertEqual(result["phase"], "late")
        self.assertEqual(result["piece_band"], "sparse")
        self.assertEqual(result["hand_band"], "small")
        self.assertEqual(result["hand_units"], 3)


if __name__ == "__main__":
    unittest.main()
