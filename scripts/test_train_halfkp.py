#!/usr/bin/env python3
"""Tests for train_halfkp.py (skipped when PyTorch or NumPy is unavailable)."""
import importlib.util
import io
import os
import struct
from contextlib import contextmanager, redirect_stderr
import sys
import tempfile
import unittest
from unittest.mock import patch

HERE = os.path.dirname(os.path.abspath(__file__))
HAVE_DEPS = all(importlib.util.find_spec(m) is not None for m in ("torch", "numpy"))

if HAVE_DEPS:
    import numpy as np
    import torch

    sys.path.insert(0, HERE)
    import train_halfkp as th


def ft_section(path):
    """Feature-transformer weights of an exported nn.bin as int16 [N_IN, HALF]."""
    with open(path, "rb") as f:
        data = f.read()
    arch_len = int.from_bytes(data[8:12], "little")
    off = 12 + arch_len + 4 + th.HALF * 2
    n = th.N_IN * th.HALF
    return np.frombuffer(data, dtype="<i2", count=n, offset=off).reshape(th.N_IN, th.HALF)


@unittest.skipUnless(HAVE_DEPS, "torch and numpy are required")
class FactorizerTest(unittest.TestCase):
    def setUp(self):
        torch.manual_seed(3)
        self.net = th.Net(24, fact=True)
        with torch.no_grad():
            self.net.ftp.weight.uniform_(-0.05, 0.05)
            self.net.ftp.weight[th.PIECE].zero_()

    def test_rows_add_the_piece_row_and_keep_padding_zero(self):
        idx = torch.tensor([[5 * th.PIECE + 7, th.PAD]])
        rows = self.net.rows(idx)
        expect = self.net.ft.weight[5 * th.PIECE + 7] + self.net.ftp.weight[7]
        self.assertTrue(torch.allclose(rows[0, 0], expect))
        self.assertTrue(torch.equal(rows[0, 1], torch.zeros(th.HALF)))

    def test_export_folds_the_piece_rows_into_every_king(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "net.bin")
            th.export(self.net, path, 24, "test")
            ft = ft_section(path)
        w = self.net.ft.weight.detach().double().numpy()[: th.N_IN]
        p = self.net.ftp.weight.detach().double().numpy()[: th.PIECE]
        for king, piece in ((0, 0), (40, 1000), (80, th.PIECE - 1)):
            row = king * th.PIECE + piece
            expect = np.clip(np.rint((w[row] + p[piece]) * 127), -32768, 32767)
            self.assertTrue(np.array_equal(ft[row], expect.astype(np.int16)), (king, piece))

    def test_folded_limit_matches_rows_export_and_plain_initialization(self):
        with torch.no_grad():
            self.net.ft.weight[0, :6] = torch.tensor([2, 4, 4, -2, -4, -4])
            self.net.ftp.weight[0, :6] = torch.tensor([3, 2, 4, -3, -2, -4])
        self.net.clamp_(24)  # Each term is legal, even when its folded sum is not.
        idx = torch.tensor([0, th.PAD])
        expected = torch.tensor([5, 6, 6, -5, -6, -6], dtype=torch.float32)
        rows = self.net.rows(idx)
        self.assertTrue(torch.equal(rows[0, :6], expected))
        self.assertTrue(torch.equal(rows[1], torch.zeros(th.HALF)))

        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "net.bin")
            th.export(self.net, path, 24, "folded limit")
            integer_rows = ft_section(path)[0, :6].copy()
            self.assertTrue(np.array_equal(integer_rows, expected.numpy().astype(np.int16) * 127))

            plain = th.Net(24, fact=False)
            th.load_init(plain, self.net.state_dict())
            self.assertTrue(torch.equal(plain.rows(idx), rows))
            th.export(plain, path, 24, "plain folded limit")
            self.assertTrue(np.array_equal(ft_section(path)[0, :6], integer_rows))

    def test_plain_model_loads_a_factorized_state_folded(self):
        plain = th.Net(24, fact=False)
        th.load_init(plain, self.net.state_dict())
        us = torch.tensor([[3 * th.PIECE + 11, 70 * th.PIECE + 1500, th.PAD]])
        self.assertTrue(torch.allclose(plain.rows(us), self.net.rows(us), atol=1e-6))

    def test_factorized_model_loads_a_plain_state_with_zero_piece_rows(self):
        plain = th.Net(24, fact=False)
        fact = th.Net(24, fact=True)
        th.load_init(fact, plain.state_dict())
        self.assertTrue(torch.equal(fact.ftp.weight, torch.zeros_like(fact.ftp.weight)))
        us = torch.tensor([[12 * th.PIECE + 3]])
        self.assertTrue(torch.allclose(fact.rows(us), plain.rows(us)))


@unittest.skipUnless(HAVE_DEPS, "torch and numpy are required")
class MappedDatasetTest(unittest.TestCase):
    @staticmethod
    def write_records(path, scores):
        records = np.zeros(len(scores), dtype=th.REC)
        records["score"] = scores
        records.tofile(path)

    def test_multiple_shards_remain_mapped_and_take_preserves_order(self):
        with tempfile.TemporaryDirectory() as tmp:
            first = os.path.join(tmp, "first.bin")
            second = os.path.join(tmp, "second.bin")
            self.write_records(first, [10, 11])
            self.write_records(second, [20, 21, 22])

            records = th.MappedRecords([first, second])
            batch = records.take(np.array([4, 0, 2, 1], dtype=np.int64))

            self.assertEqual(len(records), 5)
            self.assertTrue(all(isinstance(part, np.memmap) for part in records.parts))
            self.assertEqual(batch["score"].tolist(), [22, 10, 20, 11])

    def test_rejects_truncated_and_empty_datasets(self):
        with tempfile.TemporaryDirectory() as tmp:
            truncated = os.path.join(tmp, "truncated.bin")
            empty = os.path.join(tmp, "empty.bin")
            with open(truncated, "wb") as f:
                f.write(b"not-a-record")
            open(empty, "wb").close()

            with self.assertRaisesRegex(ValueError, "multiple"):
                th.MappedRecords([truncated])
            with self.assertRaisesRegex(ValueError, "no records"):
                th.MappedRecords([empty])

    def test_affine_permutation_is_deterministic_and_bijective(self):
        ranks = np.arange(997, dtype=np.int64)
        first = th.AffinePermutation(len(ranks), 17).take(ranks)
        second = th.AffinePermutation(len(ranks), 17).take(ranks)
        other = th.AffinePermutation(len(ranks), 18).take(ranks)

        self.assertTrue(np.array_equal(first, second))
        self.assertEqual(np.unique(first).size, len(ranks))
        self.assertFalse(np.array_equal(first, other))

    def test_train_and_validation_selections_are_disjoint(self):
        total, held_out = 101, 13
        validation = th.RecordSelection(total, 0, held_out, seed=23)
        training = th.RecordSelection(total, held_out, total - held_out, seed=23)
        val_idx = set(validation.take(np.arange(held_out)).tolist())
        train_idx = set(training.take(np.arange(total - held_out)).tolist())

        self.assertFalse(val_idx & train_idx)
        self.assertEqual(val_idx | train_idx, set(range(total)))

    def test_batch_ranks_allocates_only_one_batch(self):
        batch = th.batch_ranks(step=1234, batch_size=8192, count=20_000_000)
        self.assertEqual(len(batch), 8192)
        self.assertEqual(batch[0], 1234 * 8192)

    def test_checkpoint_order_rejects_unsafe_mid_epoch_legacy_resume(self):
        expected = th.data_order_contract(seed=1, batch_size=8192, train_count=20_000_000)
        with self.assertRaisesRegex(ValueError, "predates.*mid-epoch"):
            th.validate_checkpoint_order({"step": 12}, expected)
        th.validate_checkpoint_order({"step": 0}, expected)

    def test_checkpoint_order_requires_the_same_bounded_shuffle_contract(self):
        expected = th.data_order_contract(seed=1, batch_size=8192, train_count=100)
        th.validate_checkpoint_order({"step": 3, "data_order": expected}, expected)
        changed = th.data_order_contract(seed=2, batch_size=8192, train_count=100)
        with self.assertRaisesRegex(ValueError, "data-order mismatch"):
            th.validate_checkpoint_order({"step": 3, "data_order": changed}, expected)


@unittest.skipUnless(HAVE_DEPS, "torch and numpy are required")
class OutputSafetyTest(unittest.TestCase):
    def test_rejects_direct_input_overwrite(self):
        with tempfile.TemporaryDirectory() as tmp:
            data = os.path.join(tmp, "train.bin")
            open(data, "wb").close()
            with self.assertRaisesRegex(ValueError, "output aliases input"):
                th.validate_io_paths([data], [data])

    def test_rejects_hardlink_and_symlink_aliases(self):
        with tempfile.TemporaryDirectory() as tmp:
            data = os.path.join(tmp, "train.bin")
            hardlink = os.path.join(tmp, "hardlink.bin")
            symlink = os.path.join(tmp, "symlink.bin")
            open(data, "wb").close()
            os.link(data, hardlink)
            os.symlink(data, symlink)
            for output in (hardlink, symlink):
                with self.subTest(output=output):
                    with self.assertRaisesRegex(ValueError, "output aliases input"):
                        th.validate_io_paths([data], [output])

    def test_atomic_replace_preserves_existing_output_on_failure(self):
        with tempfile.TemporaryDirectory() as tmp:
            output = os.path.join(tmp, "network.bin")
            with open(output, "wb") as f:
                f.write(b"previous")

            def fail(temporary):
                with open(temporary, "wb") as f:
                    f.write(b"partial")
                raise RuntimeError("injected failure")

            with self.assertRaisesRegex(RuntimeError, "injected failure"):
                th._atomic_replace(output, fail)
            with open(output, "rb") as f:
                self.assertEqual(f.read(), b"previous")
            self.assertFalse(os.path.exists(th._temporary_sibling(output)))


@unittest.skipUnless(HAVE_DEPS, "torch and numpy are required")
class QuantizedWarmStartTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        torch.set_num_threads(1)
        cls.tmp = tempfile.TemporaryDirectory()
        cls.path = os.path.join(cls.tmp.name, "synthetic.bin")
        cls.arch = b"synthetic complete parameter roundtrip"
        cls.offsets = {}
        with open(cls.path, "wb") as stream:
            stream.write(struct.pack("<III", th.VERSION, th.FILE_HASH, len(cls.arch)))
            stream.write(cls.arch)
            stream.write(struct.pack("<I", th.TRANSFORMER_HASH))
            cls.offsets["ft_bias"] = stream.tell()
            np.linspace(-32768, 32767, th.HALF).astype("<i2").tofile(stream)
            cls.offsets["ft"] = stream.tell()
            for first in range(0, th.N_IN * th.HALF, 65536):
                end = min(first + 65536, th.N_IN * th.HALF)
                ((np.arange(first, end, dtype=np.int64) % 1525) - 762).astype("<i2").tofile(stream)
            cls.offsets["network_hash"] = stream.tell()
            stream.write(struct.pack("<I", th.NETWORK_HASH))
            for name, inputs in (("l1", 2 * th.HALF), ("l2", th.HIDDEN)):
                cls.offsets[name + "_bias"] = stream.tell()
                (np.arange(th.HIDDEN, dtype=np.int32) * 17123 - 10000).astype("<i4").tofile(stream)
                cls.offsets[name + "_weights"] = stream.tell()
                ((np.arange(th.HIDDEN * inputs, dtype=np.int64) % 255) - 127).astype("i1").tofile(stream)
            cls.offsets["out_bias"] = stream.tell()
            stream.write(struct.pack("<i", 12345))
            cls.offsets["out_weights"] = stream.tell()
            np.linspace(-127, 127, th.HIDDEN).astype("i1").tofile(stream)
        cls.net = th.Net(24, fact=True)

    @classmethod
    def tearDownClass(cls):
        del cls.net
        cls.tmp.cleanup()

    @contextmanager
    def changed_bytes(self, offset, replacement):
        with open(self.path, "r+b") as stream:
            stream.seek(offset)
            previous = stream.read(len(replacement))
            stream.seek(offset)
            stream.write(replacement)
        try:
            yield
        finally:
            with open(self.path, "r+b") as stream:
                stream.seek(offset)
                stream.write(previous)

    def test_complete_integer_parameters_roundtrip_and_clamp_unchanged(self):
        with torch.no_grad():
            self.net.ftp.weight.fill_(0.75)
            self.net.ft.weight[th.PAD].fill_(0.5)
        metadata = th.load_init_bin(self.net, self.path, 24)
        self.assertEqual(metadata["architecture_descriptor"], self.arch.decode())
        self.assertIn("fresh-optimizer", metadata["initialization"])
        self.assertEqual(torch.count_nonzero(self.net.ftp.weight).item(), 0)
        self.assertEqual(torch.count_nonzero(self.net.ft.weight[th.PAD]).item(), 0)
        self.net.clamp_(24)
        raw = np.memmap(self.path, mode="r", dtype="u1")
        def verify(parameter, key, dtype, count, scale, shift=0):
            expected = np.frombuffer(raw, dtype=dtype, count=count, offset=self.offsets[key])
            actual = parameter.detach().numpy().reshape(-1)
            for first in range(0, count, 65536):
                values = actual[first:first + 65536].astype(np.float64)
                encoded = np.rint(values * scale + shift)
                self.assertTrue(np.array_equal(encoded, expected[first:first + 65536]), key)
        verify(self.net.ft_bias, "ft_bias", "<i2", th.HALF, 127)
        verify(self.net.ft.weight[:th.N_IN], "ft", "<i2", th.N_IN * th.HALF, 127)
        for name, layer in (("l1", self.net.l1), ("l2", self.net.l2)):
            verify(layer.bias, name + "_bias", "<i4", th.HIDDEN, 64 * 127, 32)
            verify(layer.weight, name + "_weights", "i1", layer.weight.numel(), 64)
        verify(self.net.out.bias, "out_bias", "<i4", 1, 24)
        verify(self.net.out.weight, "out_weights", "i1", th.HIDDEN, 24 / 127)

    def test_rejects_version_and_all_structural_hashes(self):
        for offset in (0, 4, 12 + len(self.arch), self.offsets["network_hash"]):
            with self.subTest(offset=offset), self.changed_bytes(offset, struct.pack("<I", 0)):
                with self.assertRaisesRegex(ValueError, "version|hash"):
                    th.load_init_bin(self.net, self.path, 24)

    def test_rejects_architecture_length_and_invalid_utf8(self):
        with self.changed_bytes(8, struct.pack("<I", 4097)):
            with self.assertRaisesRegex(ValueError, "4096"):
                th.load_init_bin(self.net, self.path, 24)
        with self.changed_bytes(12, b"\xff"):
            with self.assertRaisesRegex(ValueError, "UTF-8"):
                th.load_init_bin(self.net, self.path, 24)

    def test_rejects_truncated_and_trailing_images(self):
        bad = os.path.join(self.tmp.name, "bad.bin")
        for length in (4, os.path.getsize(self.path) - 1, os.path.getsize(self.path) + 1):
            with self.subTest(length=length):
                with open(bad, "wb") as stream:
                    stream.write(struct.pack("<III", th.VERSION, th.FILE_HASH, len(self.arch)))
                    stream.truncate(length)
                with self.assertRaisesRegex(ValueError, "truncated|length"):
                    th.load_init_bin(self.net, bad, 24)

    def test_rejects_clipped_or_nonrepresentable_parameters_without_partial_load(self):
        cases = [("ft", struct.pack("<h", 763), "clamp"),
                 ("l1_weights", b"\x80", "clamp"),
                 ("l2_weights", b"\x80", "clamp"),
                 ("out_weights", b"\x80", "clamp"),
                 ("l2_bias", struct.pack("<i", 2147483647), "representable")]
        for key, value, message in cases:
            before = self.net.ft_bias.detach().clone()
            with self.subTest(key=key), self.changed_bytes(self.offsets[key], value):
                with self.assertRaisesRegex(ValueError, message):
                    th.load_init_bin(self.net, self.path, 24)
            self.assertTrue(torch.equal(before, self.net.ft_bias))

    def test_scale_is_explicit_positive_and_bounded(self):
        for value in (0, -1, 129, 24.0, True):
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, "FV scale"):
                th.load_init_bin(self.net, self.path, value)
        with self.assertRaisesRegex(ValueError, "scale differ"):
            th.load_init_bin(self.net, self.path, 16)

    def test_warm_start_is_exclusive_with_float_init_or_existing_resume(self):
        with self.assertRaisesRegex(ValueError, "mutually exclusive"):
            th.validate_initialization("old.pt", self.path, None)
        with self.assertRaisesRegex(ValueError, "resume checkpoint"):
            th.validate_initialization(None, self.path, self.path)
        th.validate_initialization(None, self.path, os.path.join(self.tmp.name, "new.ckpt"))

    def test_cli_requires_explicit_scale_before_accessing_inputs(self):
        argv = ["train_halfkp.py", "--data", "/missing/data.bin", "--out", "/missing/out.bin",
                "--init-bin", "/missing/parent.bin"]
        with patch.object(sys, "argv", argv), patch.object(th, "validate_io_paths") as paths:
            with redirect_stderr(io.StringIO()) as error, self.assertRaises(SystemExit) as stopped:
                th.main()
            self.assertEqual(stopped.exception.code, 2)
            self.assertIn("explicitly supplied --fv-scale", error.getvalue())
            paths.assert_not_called()

    def test_cli_accepts_equals_scale_before_input_access(self):
        argv = ["train_halfkp.py", "--data", "/missing/data.bin", "--out", "/missing/out.bin",
                "--init-bin", "/missing/parent.bin", "--fv-scale=24"]
        with patch.object(sys, "argv", argv), patch.object(th, "validate_fv_scale", wraps=th.validate_fv_scale) as scale:
            with patch.object(th, "validate_io_paths", side_effect=RuntimeError("before input access")):
                with self.assertRaisesRegex(RuntimeError, "before input access"):
                    th.main()
            scale.assert_called_once_with(24)

    def test_cli_retains_default_scale_for_existing_modes(self):
        argv = ["train_halfkp.py", "--data", "/missing/data.bin", "--out", "/missing/out.bin"]
        with patch.object(sys, "argv", argv), patch.object(th, "validate_fv_scale", wraps=th.validate_fv_scale) as scale:
            with patch.object(th, "validate_io_paths", side_effect=RuntimeError("before input access")):
                with self.assertRaisesRegex(RuntimeError, "before input access"):
                    th.main()
            scale.assert_called_once_with(24)

    def test_bin_input_cannot_alias_any_output(self):
        with self.assertRaisesRegex(ValueError, "output aliases input"):
            th.validate_io_paths([self.path], [self.path])
        alias = os.path.join(self.tmp.name, "alias.bin")
        os.link(self.path, alias)
        try:
            with self.assertRaisesRegex(ValueError, "output aliases input"):
                th.validate_io_paths([self.path], [alias])
        finally:
            os.unlink(alias)


if __name__ == "__main__":
    unittest.main()
