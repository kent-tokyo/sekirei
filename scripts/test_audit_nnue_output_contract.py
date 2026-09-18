#!/usr/bin/env python3
"""Unit tests for the NNUE output/teacher contract audit."""

from __future__ import annotations

import importlib.util
import json
import struct
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("contract", ROOT / "audit_nnue_output_contract.py")
assert SPEC and SPEC.loader
CONTRACT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CONTRACT)


def fixture_weights(path: Path, out: list[float], bias: float) -> None:
    prefix = b"\0" * (8 + CONTRACT.INPUT * CONTRACT.L1 * 2 + CONTRACT.L1 * 2 + 2 * CONTRACT.L1 * CONTRACT.L2 * 4 + CONTRACT.L2 * 4)
    path.write_bytes(CONTRACT.MAGIC + prefix[8:] + struct.pack(f"<{CONTRACT.L2 + 1}f", *out, bias))


class ContractAuditTests(unittest.TestCase):
    def test_bound_and_label_cap_detect_unrepresentable_target(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            weights, cache = root / "weights.bin", root / "cache.jsonl"
            fixture_weights(weights, [-1.0] + [0.0] * 31, 0.0)
            cache.write_text("{\"score_cp\": -1200}\n{\"score_cp\": 1200}\n", encoding="utf-8")
            document = CONTRACT.audit(weights, cache, 1200.0)
            self.assertEqual(document["model"]["theoretical_min_cp"], -127.0 / 64.0)
            self.assertFalse(document["conclusion"]["checkpoint_envelope_contains_all_capped_labels"])
            self.assertGreater(document["conclusion"]["lower_shortfall_cp"], 1_000.0)
            self.assertGreater(document["conclusion"]["upper_shortfall_cp"], 1_000.0)
            self.assertEqual(document["conclusion"]["action"], "hold_checkpoint_from_strength_gate")

    def test_label_summary_uses_training_cap(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            weights, cache = root / "weights.bin", root / "cache.jsonl"
            fixture_weights(weights, [-1000.0, 1000.0] + [0.0] * 30, 0.0)
            cache.write_text("\n".join(json.dumps({"score_cp": value}) for value in (-9_999, 0, 9_999)) + "\n", encoding="utf-8")
            document = CONTRACT.audit(weights, cache, 600.0)
            self.assertEqual(document["labels"]["at_or_beyond_cap"], 2)
            self.assertEqual(document["labels"]["capped_min_cp"], -600.0)
            self.assertEqual(document["labels"]["capped_max_cp"], 600.0)
            self.assertTrue(document["conclusion"]["checkpoint_envelope_contains_all_capped_labels"])


if __name__ == "__main__":
    unittest.main()
