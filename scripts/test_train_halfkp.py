#!/usr/bin/env python3
"""Tests for train_halfkp.py (skipped when PyTorch or NumPy is unavailable)."""
import importlib.util
import os
import sys
import tempfile
import unittest

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


if __name__ == "__main__":
    unittest.main()
