#!/usr/bin/env python3

import importlib.util
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "summarize_self_distill", ROOT / "summarize_self_distill.py"
)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


class SelfDistillSummaryTest(unittest.TestCase):
    def test_validation_losses_and_earliest_tie(self):
        with tempfile.TemporaryDirectory() as tmp:
            log = Path(tmp) / "training.log"
            log.write_text(
                "Epoch 1/3 — lr = 0.001000\n"
                "  valid: loss=12.5  cp_mse=20\n"
                "Epoch 2/3 — lr = 0.001000\n"
                "  valid: loss=10.0  cp_mse=18\n"
                "Epoch 3/3 — lr = 0.001000\n"
                "  valid: loss=10.0  cp_mse=17\n",
                encoding="utf-8",
            )
            losses = MODULE.validation_losses(log)
            self.assertEqual(losses, {1: 12.5, 2: 10.0, 3: 10.0})
            self.assertEqual(min(losses.items(), key=lambda item: (item[1], item[0]))[0], 2)

    def test_validation_before_epoch_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            log = Path(tmp) / "training.log"
            log.write_text("  valid: loss=1.0\n", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "precedes epoch"):
                MODULE.validation_losses(log)


if __name__ == "__main__":
    unittest.main()
