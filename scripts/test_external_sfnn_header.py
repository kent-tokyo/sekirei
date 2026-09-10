#!/usr/bin/env python3
"""Regression tests for bounded external SFNN header inspection."""

import importlib.util
import struct
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("sfnn_header", ROOT / "inspect_external_sfnn.py")
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


ARCH = "ModelType=SFNNWithoutPsqt;Features=HalfKP;Network=1536-15-32{LayerStack=9}"


def write_header(path: Path, architecture: str = ARCH, body: bytes = b"") -> None:
    encoded = architecture.encode("utf-8")
    path.write_bytes(struct.pack("<III", 1, 0x1234ABCD, len(encoded)) + encoded + body)


class SfnnHeaderTests(unittest.TestCase):
    def test_valid_header_is_inspected_without_reading_weight_body(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "nn.bin"
            write_header(path)
            result = MODULE.inspect(path)
            self.assertEqual(result["version"], 1)
            self.assertEqual(result["hash"], "1234abcd")
            self.assertEqual(result["layer_stacks"], 9)

    def test_non_sfnn_architecture_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "nn.bin"
            write_header(path, "Features=HalfKP;Network=1536-15-32")
            with self.assertRaises(MODULE.SfnnHeaderError):
                MODULE.inspect(path)

    def test_truncated_architecture_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "nn.bin"
            path.write_bytes(struct.pack("<III", 1, 2, 100) + b"short")
            with self.assertRaises(MODULE.SfnnHeaderError):
                MODULE.inspect(path)

    def test_invalid_layer_stack_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "nn.bin"
            write_header(path, ARCH.replace("LayerStack=9", "LayerStack=0"))
            with self.assertRaises(MODULE.SfnnHeaderError):
                MODULE.inspect(path)


if __name__ == "__main__":
    unittest.main()
