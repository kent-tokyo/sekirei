#!/usr/bin/env python3
"""Validate the native parity driver's input binding without engine processes."""
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
from verify_halfkp_native_parity import PACKED, PADDING, native_scores, unpack_features


class NativeParityInputTest(unittest.TestCase):
    SFEN = "4k4/9/9/9/9/9/9/9/4K4 w - 1"

    def record(self, stm=1, king_us=5, king_them=7, first=123):
        return PACKED.pack(stm, 0, 0, king_us, king_them, first, *([1548] * 37), *([1548] * 38))

    def test_packed_indices_keep_side_order_and_padding(self):
        row = unpack_features(self.record(), [self.SFEN])[0]
        self.assertEqual(row["us"][0], 5 * 1548 + 123)
        self.assertEqual(row["us"][1:], [PADDING] * 37)
        self.assertEqual(row["them"], [PADDING] * 38)

    def test_mismatched_or_malformed_packed_rows_are_rejected(self):
        for raw in (self.record()[:-1], self.record(stm=0), self.record(king_us=81), self.record(first=1549)):
            with self.subTest(raw_length=len(raw)), self.assertRaises(ValueError):
                unpack_features(raw, [self.SFEN])

    def test_native_scores_bind_each_output_to_the_exact_sfen(self):
        self.assertEqual(native_scores(self.SFEN + "\t-42\n", [self.SFEN]), [-42])
        for payload in ("", self.SFEN + "\t-42\nextra\t1\n", self.SFEN.replace(" w ", " b ") + "\t-42\n"):
            with self.subTest(payload=payload), self.assertRaises(ValueError):
                native_scores(payload, [self.SFEN])


if __name__ == "__main__":
    unittest.main()
