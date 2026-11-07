"""Synthetic contracts for the opt-in, native-integer SEKHK001 trainer."""

import math
import tempfile
import unittest
from pathlib import Path

import numpy as np
import torch

from train_halfkp_plus_kings import (
    FILE_BYTES,
    HEADER,
    INPUTS,
    LEGACY_INPUTS,
    PAD,
    PIECE,
    REC,
    IntegerNetwork,
    Net,
    indices,
    main,
    squared_prediction_error,
    synthetic_init,
    write_network,
)
from train_halfkp import Net as LegacyNet, indices as legacy_indices


def record(own=8, opposite=26):
    rows = np.zeros(1, dtype=REC)
    rows["kus"] = own
    rows["kthem"] = opposite
    rows["fus"][:] = PIECE
    rows["fthem"][:] = PIECE
    rows["fus"][0, 0] = 0
    rows["fthem"][0, 0] = 1
    return rows


class PlusKingsTrainerContracts(unittest.TestCase):
    def test_format_and_both_perspective_king_mapping(self):
        self.assertEqual(len(HEADER), 40)
        self.assertEqual(FILE_BYTES, 70_935_372)
        self.assertEqual(REC.itemsize, 158)
        self.assertEqual(INPUTS, LEGACY_INPUTS + 81 * 162)
        us, them = indices(record())
        self.assertEqual(us[0, 0], 8 * PIECE)
        self.assertEqual(them[0, 0], 26 * PIECE + 1)
        self.assertEqual(us[0, 1], PAD)
        self.assertEqual(us[0, 38], LEGACY_INPUTS + 8 * 162 + 8)
        self.assertEqual(us[0, 39], LEGACY_INPUTS + 8 * 162 + 81 + 80 - 26)
        self.assertEqual(them[0, 38], LEGACY_INPUTS + 26 * 162 + 26)
        self.assertEqual(them[0, 39], LEGACY_INPUTS + 26 * 162 + 81 + 80 - 8)

    def test_packed_invalid_values_fail_closed(self):
        for field, value in (("stm", 2), ("res", 2), ("kus", 81), ("kthem", 81)):
            with self.subTest(field=field):
                rows = record()
                rows[field] = value
                with self.assertRaisesRegex(ValueError, "invalid packed"):
                    indices(rows)
        rows = record()
        rows["fus"][0, 1] = PIECE + 1
        with self.assertRaisesRegex(ValueError, "invalid packed"):
            indices(rows)

    def test_loss_is_the_baseline_sigmoid_teacher_result_recipe(self):
        rows = record()
        rows["score"] = 200
        rows["res"] = 1

        class FixedPrediction:
            def __call__(self, us, them):
                self_shapes = (us.shape, them.shape)
                assert self_shapes == ((1, 40), (1, 40))
                return torch.tensor([100.0], dtype=torch.float32)

        loss = squared_prediction_error(FixedPrediction(), rows, 0.25, 400)
        target = 0.25 * (1 / (1 + math.exp(-200 / 400))) + 0.75
        expected = ((1 / (1 + math.exp(-100 / 400))) - target) ** 2
        self.assertAlmostEqual(float(loss[0]), expected, places=7)

    def test_float32_export_matches_independent_integer_forward(self):
        torch.set_num_threads(1)
        net = Net(24)
        synthetic_init(net, 42)
        self.assertEqual(net.ft.weight.dtype, torch.float32)
        self.assertFalse(net.ft.sparse)
        rows = np.concatenate((record(8, 26), record(40, 2)))
        us, them = indices(rows)
        predicted = net(torch.from_numpy(us), torch.from_numpy(them)).detach().numpy()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "synthetic.sekhk"
            with path.open("wb") as stream:
                write_network(net, stream)
            self.assertEqual(path.stat().st_size, FILE_BYTES)
            oracle = IntegerNetwork(path, 24)
            actual = [oracle.trace(us[index], them[index])["score"] for index in range(len(rows))]
            self.assertEqual(actual, predicted.astype(int).tolist())

    def test_zero_king_rows_preserve_legacy_first_adam_update(self):
        torch.set_num_threads(1)
        plus = Net(24)
        synthetic_init(plus, 42)
        with torch.no_grad():
            plus.ft.weight[LEGACY_INPUTS:INPUTS].zero_()
        legacy = LegacyNet(fv_scale=24, fact=False)
        with torch.no_grad():
            legacy.ft.weight[:LEGACY_INPUTS].copy_(plus.ft.weight[:LEGACY_INPUTS])
            legacy.ft.weight[LEGACY_INPUTS].zero_()
            legacy.ft_bias.copy_(plus.ft_bias)
            for old_layer, new_layer in ((legacy.l1, plus.l1), (legacy.l2, plus.l2),
                                          (legacy.out, plus.out)):
                old_layer.weight.copy_(new_layer.weight)
                old_layer.bias.copy_(new_layer.bias)
        rows = np.concatenate((record(8, 26), record(40, 2)))
        rows["score"] = [200, -100]
        rows["res"] = [1, -1]
        old_us, old_them = (torch.from_numpy(x) for x in legacy_indices(rows))
        new_us, new_them = (torch.from_numpy(x) for x in indices(rows))
        self.assertTrue(torch.equal(legacy(old_us, old_them), plus(new_us, new_them)))
        old_optimizer = torch.optim.Adam(legacy.parameters(), lr=0.001)
        new_optimizer = torch.optim.Adam(plus.parameters(), lr=0.001)
        old_scheduler = torch.optim.lr_scheduler.CosineAnnealingLR(old_optimizer, T_max=1, eta_min=0.00005)
        new_scheduler = torch.optim.lr_scheduler.CosineAnnealingLR(new_optimizer, T_max=1, eta_min=0.00005)
        old_target = torch.sigmoid(torch.tensor([200.0, -100.0]) / 400)
        old_loss = (torch.sigmoid(legacy(old_us, old_them) / 400) - old_target).square().mean()
        new_loss = squared_prediction_error(plus, rows, 1.0, 400).mean()
        self.assertTrue(torch.equal(old_loss, new_loss))
        old_loss.backward()
        new_loss.backward()
        old_optimizer.step()
        new_optimizer.step()
        old_scheduler.step()
        new_scheduler.step()
        legacy.clamp_(24)
        plus.clamp_()
        self.assertTrue(torch.equal(legacy.ft.weight[:LEGACY_INPUTS], plus.ft.weight[:LEGACY_INPUTS]))
        self.assertTrue(torch.equal(legacy.ft_bias, plus.ft_bias))
        for old_layer, new_layer in ((legacy.l1, plus.l1), (legacy.l2, plus.l2),
                                      (legacy.out, plus.out)):
            self.assertTrue(torch.equal(old_layer.weight, new_layer.weight))
            self.assertTrue(torch.equal(old_layer.bias, new_layer.bias))
        self.assertEqual(old_scheduler.get_last_lr(), new_scheduler.get_last_lr())

    def test_unbounded_or_missing_training_contract_is_rejected(self):
        base = ["train", "--output-dir", "/private/tmp/uncreated-sekirei-trainer-test",
                "--run-id", "synthetic-check", "--fv-scale", "24",
                "--synthetic-seed", "1", "--source-kind", "synthetic",
                "--source-declaration", "synthetic-only"]
        with self.assertRaisesRegex(ValueError, "train requires separate packs"):
            main(base)
        with self.assertRaisesRegex(ValueError, "lam must be finite"):
            main(base + ["--lam", "nan"])


if __name__ == "__main__":
    unittest.main()
