import tempfile
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from nps_threads import (
    bind_yaneuraou_evaluator, check_yaneuraou_evaluator,
    file_fingerprint, check_file_fingerprint, reject_evaluator_overrides,
    yaneuraou_eval_dir,
)


class NativeEvaluatorBindingTest(unittest.TestCase):
    def test_requested_filename_cannot_hide_a_different_native_file(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "selected.bin").write_bytes(b"requested")
            (root / "nn.bin").write_bytes(b"other")
            with self.assertRaisesRegex(ValueError, "differs"):
                bind_yaneuraou_evaluator(root / "selected.bin", 24)
            (root / "nn.bin").write_bytes(b"requested")
            binding = bind_yaneuraou_evaluator(root / "selected.bin", 24)
            self.assertEqual(binding["requested"]["sha256"], binding["native_nnue"]["sha256"])
            check_yaneuraou_evaluator(binding)

    def test_missing_or_nonregular_native_file_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "selected.bin").write_bytes(b"requested")
            with self.assertRaises(FileNotFoundError):
                bind_yaneuraou_evaluator(root / "selected.bin", 24)
            (root / "nn.bin").mkdir()
            with self.assertRaisesRegex(ValueError, "regular"):
                bind_yaneuraou_evaluator(root / "selected.bin", 24)

    def test_symlink_parent_selects_the_requested_nn_bin_not_a_target_sibling(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); requested = root / "requested"; target = root / "target"
            requested.mkdir(); target.mkdir()
            (target / "weights.bin").write_bytes(b"right")
            (target / "nn.bin").write_bytes(b"wrong")
            (requested / "nn.bin").symlink_to(target / "weights.bin")
            binding = bind_yaneuraou_evaluator(requested / "nn.bin", 24)
            self.assertEqual(binding["eval_dir"], str(requested))
            self.assertEqual(yaneuraou_eval_dir(requested / "nn.bin"), str(requested))
            self.assertEqual(binding["native_nnue"]["resolved_path"], str((target / "weights.bin").resolve()))
            check_yaneuraou_evaluator(binding)
            (requested / "nn.bin").unlink()
            (requested / "nn.bin").symlink_to(target / "nn.bin")
            with self.assertRaisesRegex(ValueError, "changed"):
                check_yaneuraou_evaluator(binding)

    def test_mutation_of_both_matching_files_still_invalidates_binding(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "nn.bin"; path.write_bytes(b"before")
            binding = bind_yaneuraou_evaluator(path, 24)
            path.write_bytes(b"after!")
            with self.assertRaisesRegex(ValueError, "changed"):
                check_yaneuraou_evaluator(binding)

    def test_binary_fingerprint_detects_replacement(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "engine"; path.write_bytes(b"binary1")
            value = file_fingerprint(path); check_file_fingerprint(value)
            path.write_bytes(b"binary2")
            with self.assertRaisesRegex(ValueError, "changed"):
                check_file_fingerprint(value)

    def test_overrides_and_multiline_injection_are_rejected(self):
        for option in ("EvalFile=x", " EVALdir =x", "fv_SCALE=16", "NnueOutput=relative",
                       "Threads=1\nsetoption name EvalDir value bad", "X=1\0", "Threads", "=x",
                       "FV_SCALE value 16=ignored", "FV_SCALE ignored 16=ignored",
                       "EvalDir\tvalue /other=ignored", "Other VaLuE injected=x"):
            with self.subTest(option=option), self.assertRaises(ValueError):
                reject_evaluator_overrides([option])
        reject_evaluator_overrides(["Threads=4", "T_ASP_VOLATILITY=1", "BookFile=no_book", "NnueOutput=absolute", "Skill Level=0"])

    def test_scale_and_path_injection_fail_before_file_access(self):
        for scale in (0, 129, True):
            with self.subTest(scale=scale), self.assertRaisesRegex(ValueError, "FV_SCALE"):
                bind_yaneuraou_evaluator("missing/nn.bin", scale)
        for path in ("nn.bin\nquit", "nn.bin\0", " nn.bin", ""):
            with self.subTest(path=path), self.assertRaises(ValueError):
                yaneuraou_eval_dir(path)


if __name__ == "__main__":
    unittest.main()
