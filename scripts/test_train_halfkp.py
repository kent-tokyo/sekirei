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


if __name__ == "__main__":
    unittest.main()
