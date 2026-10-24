#!/usr/bin/env python3
"""Bounded, exact integer HalfKP diagnostic regression tests."""
import contextlib
import io
import os
from pathlib import Path
import struct
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import compare_halfkp_parameters as diff


def fixture(path, descriptor=b"synthetic", changes=()):
    with path.open("wb") as stream:
        stream.write(struct.pack("<III", diff.VERSION, diff.FILE_HASH, len(descriptor)))
        stream.write(descriptor)
        stream.write(struct.pack("<I", diff.TRANSFORMER_HASH))
        offsets = {}
        for name, code, count in diff.SECTIONS:
            if name == "l1_bias":
                stream.write(struct.pack("<I", diff.NETWORK_HASH))
            offsets[name] = stream.tell()
            stream.seek(struct.calcsize("<" + code) * count, os.SEEK_CUR)
        stream.truncate()
        for name, index, value in changes:
            code = next(code for key, code, count in diff.SECTIONS if key == name)
            stream.seek(offsets[name] + index * struct.calcsize("<" + code))
            stream.write(struct.pack("<" + code, value))
    return offsets


class ParameterDiffTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.left, self.right = self.root / "left.bin", self.root / "right.bin"
        self.offsets = fixture(self.left)

    def test_descriptor_difference_and_known_bias_deltas_leave_ft_unchanged(self):
        fixture(self.right, b"a longer descriptor", [("l1_bias", 0, 7), ("l1_bias", 31, -3)])
        report = diff.compare_parameters(self.left, self.right, chunk_values=16384)
        sections = {s["name"]: s for s in report["sections"]}
        self.assertEqual(report["changed"], 2)
        self.assertFalse(report["parameters_equal"])
        self.assertNotEqual(report["left"]["bytes"], report["right"]["bytes"])
        self.assertEqual(sections["ft_weight"]["changed"], 0)
        self.assertEqual(sections["ft_weight"]["total"], 32099328)
        self.assertIsNone(sections["ft_weight"]["minimum_changed_delta"])
        self.assertEqual(sections["l1_bias"]["minimum_changed_delta"], -3)
        self.assertEqual(sections["l1_bias"]["maximum_changed_delta"], 7)
        self.assertEqual(sections["l1_bias"]["absolute_delta_sum"], 10)
        self.assertEqual(sections["l1_bias"]["changed_percent"], 6.25)

    def test_signed_extremes_do_not_overflow_and_changes_cross_chunks(self):
        fixture(self.right, changes=[("ft_bias", 1, -32768),
                                     ("ft_weight", 65535, 32767),
                                     ("ft_weight", 65536, -32768),
                                     ("l1_weight", 17, -128),
                                     ("out_bias", 0, 2147483647)])
        with self.left.open("r+b") as stream:
            stream.seek(self.offsets["out_bias"])
            stream.write(struct.pack("<i", -2147483648))
        result = diff.compare_parameters(self.left, self.right)
        sections = {s["name"]: s for s in result["sections"]}
        self.assertEqual(sections["ft_bias"]["minimum_changed_delta"], -32768)
        self.assertEqual(sections["ft_weight"]["changed"], 2)
        self.assertEqual(sections["ft_weight"]["minimum_changed_delta"], -32768)
        self.assertEqual(sections["ft_weight"]["maximum_changed_delta"], 32767)
        self.assertEqual(sections["l1_weight"]["minimum_changed_delta"], -128)
        self.assertEqual(sections["out_bias"]["maximum_changed_delta"], 4294967295)

    def test_same_input_is_allowed_and_not_a_strength_verdict(self):
        report = diff.compare_parameters(self.left, self.left)
        self.assertTrue(report["parameters_equal"])
        self.assertEqual(report["changed"], 0)
        self.assertIn("no playing-strength", report["interpretation"])

    def test_header_descriptor_and_structural_hash_rejections(self):
        mutations = [(0, struct.pack("<I", 0)), (4, struct.pack("<I", 0)),
                     (8, struct.pack("<I", 4097)), (12, b"\xff"),
                     (21, struct.pack("<I", 0)),
                     (self.offsets["l1_bias"] - 4, struct.pack("<I", 0))]
        for offset, replacement in mutations:
            with self.subTest(offset=offset):
                fixture(self.right)
                with self.right.open("r+b") as stream:
                    stream.seek(offset)
                    stream.write(replacement)
                with self.assertRaises(ValueError):
                    diff.inspect_file(self.right)

    def test_truncated_header_payload_and_trailing_bytes_are_rejected(self):
        for size in (0, 11, 22, self.left.stat().st_size - 1, self.left.stat().st_size + 1):
            with self.subTest(size=size):
                fixture(self.right)
                with self.right.open("r+b") as stream:
                    stream.truncate(size)
                with self.assertRaises(ValueError):
                    diff.inspect_file(self.right)

    def test_invalid_chunk_sizes(self):
        for value in (0, -1, 1.5, True):
            with self.subTest(value=value), self.assertRaises(ValueError):
                diff.compare_parameters(self.left, self.left, chunk_values=value)

    def test_changed_input_is_detected(self):
        actual = diff.file_sha256
        calls = 0
        def hash_with_change(path):
            nonlocal calls
            calls += 1
            return "changed" if calls == 3 else actual(path)
        with patch.object(diff, "file_sha256", side_effect=hash_with_change):
            with self.assertRaisesRegex(ValueError, "input changed"):
                diff.compare_parameters(self.left, self.left)

    def test_new_output_is_valid_json_and_never_overwritten(self):
        output = self.root / "new" / "report.json"
        report = {"diagnostic": True}
        diff.write_new_report(output, report, (self.left,))
        self.assertEqual(output.read_text(), '{\n  "diagnostic": true\n}\n')
        with self.assertRaises(FileExistsError):
            diff.write_new_report(output, {}, (self.left,))
        self.assertEqual(list(output.parent.iterdir()), [output])

    def test_input_aliases_and_existing_broken_symlink_are_rejected(self):
        alias = self.root / "alias"
        for kind in ("same", "symlink", "hardlink"):
            with self.subTest(kind=kind):
                if kind == "same":
                    target = self.left
                elif kind == "symlink":
                    alias.symlink_to(self.left)
                    target = alias
                else:
                    os.link(self.left, alias)
                    target = alias
                with self.assertRaisesRegex(ValueError, "aliases"):
                    diff.write_new_report(target, {}, (self.left,))
                if target == alias:
                    alias.unlink()
        alias.symlink_to(self.root / "missing")
        with self.assertRaises(FileExistsError):
            diff.write_new_report(alias, {}, (self.left,))

    def test_atomic_no_replace_race_preserves_other_output(self):
        output = self.root / "report.json"
        link = os.link
        def competing_writer(source, destination):
            Path(destination).write_text("other writer")
            return link(source, destination)
        with patch.object(diff.os, "link", side_effect=competing_writer):
            with self.assertRaises(FileExistsError):
                diff.write_new_report(output, {}, (self.left,))
        self.assertEqual(output.read_text(), "other writer")
        self.assertEqual(sorted(p.name for p in self.root.iterdir()), ["left.bin", "report.json"])

    def test_serialization_failure_leaves_no_partial_artifact(self):
        output = self.root / "report.json"
        with self.assertRaises(TypeError):
            diff.write_new_report(output, {"bad": object()}, (self.left,))
        self.assertFalse(output.exists())
        self.assertEqual(list(self.root.iterdir()), [self.left])

    def test_cli_error_does_not_create_output(self):
        output = self.root / "report.json"
        self.right.write_bytes(b"broken")
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as error:
            diff.main([str(self.left), str(self.right), "--output", str(output)])
        self.assertEqual(error.exception.code, 1)
        self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
