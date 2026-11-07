#!/usr/bin/env python3
"""Small sparse nn.bin fixtures; no Torch, NumPy, engine, or real weights."""
import os
from pathlib import Path
import struct
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
from halfkp_integer_oracle import HalfKpIntegerOracle


def sparse_network(path):
    """A real format image: one active feature and a one-channel identity head."""
    arch = b"integer oracle boundary fixture"
    offsets = {}
    with open(path, "wb") as stream:
        stream.write(struct.pack("<III", 0x7AF32F16, 0x3E5AA6EE, len(arch)))
        stream.write(arch)
        offsets["ft_hash"] = stream.tell()
        stream.write(struct.pack("<I", 0x5D69D7B8))
        offsets["ft_bias"] = stream.tell()
        stream.write(struct.pack("<256h", *([0] * 256)))
        offsets["ft_weights"] = stream.tell()
        stream.write(struct.pack("<256h", 1, *([0] * 255)))
        stream.seek(offsets["ft_weights"] + 125388 * 256 * 2)
        offsets["network_hash"] = stream.tell()
        stream.write(struct.pack("<I", 0x3E5AA6EE ^ 0x5D69D7B8))
        for name, width in (("l1", 512), ("l2", 32)):
            offsets[name + "_bias"] = stream.tell()
            stream.write(struct.pack("<32i", *([32] * 32)))
            offsets[name + "_weights"] = stream.tell()
            weights = [0] * (32 * width)
            weights[0] = 64
            stream.write(struct.pack(f"<{len(weights)}b", *weights))
        offsets["out_bias"] = stream.tell()
        stream.write(struct.pack("<i", 0))
        stream.write(struct.pack("<32b", 24, *([0] * 31)))
    return offsets


class IntegerOracleTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.path = os.path.join(self.tmp.name, "fixture.bin")
        self.offsets = sparse_network(self.path)

    def tearDown(self):
        self.tmp.cleanup()

    def replace(self, offset, fmt, *values):
        with open(self.path, "r+b") as stream:
            stream.seek(offset)
            stream.write(struct.pack("<" + fmt, *values))

    def test_expected_layers_padding_and_side_order(self):
        with HalfKpIntegerOracle(self.path, 24) as oracle:
            trace = oracle.trace([0, 125388], [125388, 125388])
            self.assertEqual(trace["acc_us"][0], 1)
            self.assertEqual(trace["input"][256], 0)
            self.assertEqual(trace["l1"][0], 1)
            self.assertEqual(trace["l2"][0], 1)
            self.assertEqual(trace["raw_output"], 24)
            self.assertEqual(trace["score"], 1)
            self.assertEqual(oracle.trace([125388], [0])["score"], 0)

    def test_accumulator_wrap_and_hidden_negative_shift(self):
        self.replace(self.offsets["ft_bias"], "h", 32767)
        self.replace(self.offsets["l1_bias"], "i", -1)
        with HalfKpIntegerOracle(self.path, 24) as oracle:
            trace = oracle.trace([0], [125388])
            self.assertEqual(trace["acc_us"][0], -32768)
            self.assertEqual(trace["l1"][0], 0)

    def test_output_division_and_engine_score_clamp(self):
        for raw, expected in ((-23, 0), (-25, -1), (25, 1), (1000000, 30000), (-1000000, -30000)):
            self.replace(self.offsets["out_bias"], "i", raw)
            with self.subTest(raw=raw), HalfKpIntegerOracle(self.path, 24) as oracle:
                self.assertEqual(oracle.trace([125388], [125388])["score"], expected)

    def test_rejects_bad_features_scale_and_overflow_headroom(self):
        with HalfKpIntegerOracle(self.path, 24) as oracle:
            for values in ([], [0] * 39, [-1], [125389], [True]):
                with self.subTest(values=values), self.assertRaises(ValueError):
                    oracle.trace(values, [0])
        for scale in (0, 129, 24.0, True):
            with self.subTest(scale=scale), self.assertRaises(ValueError):
                HalfKpIntegerOracle(self.path, scale)
        self.replace(self.offsets["out_bias"], "i", 2**31 - 1)
        with self.assertRaisesRegex(ValueError, "headroom"):
            HalfKpIntegerOracle(self.path, 24)

    def test_rejects_hashes_and_trailing_data(self):
        for offset in (0, 4, self.offsets["ft_hash"], self.offsets["network_hash"]):
            self.offsets = sparse_network(self.path)
            self.replace(offset, "I", 0)
            with self.subTest(offset=offset), self.assertRaisesRegex(ValueError, "version|hash"):
                HalfKpIntegerOracle(self.path, 24)
        self.offsets = sparse_network(self.path)
        with open(self.path, "ab") as stream:
            stream.write(b"x")
        with self.assertRaisesRegex(ValueError, "length"):
            HalfKpIntegerOracle(self.path, 24)


if __name__ == "__main__":
    unittest.main()
