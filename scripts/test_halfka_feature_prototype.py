#!/usr/bin/env python3
"""Synthetic feature/format tests; optional native inference via explicit binary."""
import copy
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import halfka_feature_prototype as prototype
from halfkp_integer_oracle import HalfKpIntegerOracle

KINGS = "4k4/9/9/9/9/9/9/9/4K4 b - 1"
MOVED = "4k4/9/9/9/9/9/9/4K4/9 w - 2"
NATIVE = os.environ.get("SEKIREI_HALFKA_PROBE")


def simple_model(kind="halfka-kings"):
    rows = {}
    if kind == "halfka-kings":
        active, _ = prototype.feature_views(KINGS, kind)
        rows = {index: [0]*256 for index in active[0]}
        rows[active[0][0]][0] = 2
        rows[active[0][1]][0] = 1
    layers = []
    for width in (512, 32):
        weights = [0]*(32*width)
        weights[0] = 64
        layers.append({"bias": [32]*32, "weights": weights})
    return {"kind": kind, "fv_scale": 24, "bias": [0]*256, "rows": rows,
            "layers": layers, "output_bias": -23, "output_weights": [24]+[0]*31}


class PrototypeContractTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.path = self.root / "synthetic.skx"

    def save(self, model=None, raw=None):
        self.path.write_bytes(raw if raw is not None else prototype.encode(model or simple_model()))
        return self.path

    def test_appended_king_features_preserve_all_legacy_indices(self):
        for sfen in prototype.FIXTURES:
            baseline, stm = prototype.feature_views(sfen, "halfkp")
            extended, other_stm = prototype.feature_views(sfen, "halfka-kings")
            self.assertEqual(stm, other_stm)
            for old, new in zip(baseline, extended):
                self.assertEqual(old, [index for index in new if index < prototype.BASE_INPUT])
                self.assertEqual(len(new), len(old)+2)
                self.assertEqual(len(new), len(set(new)))

    def test_king_coordinate_golden_and_opponent_update(self):
        original, _ = prototype.feature_views(KINGS, "halfka-kings")
        self.assertEqual(original, [[132560,132633], [132560,132633]])
        moved, _ = prototype.feature_views(MOVED, "halfka-kings")
        self.assertEqual(moved[1], [132560,132634])
        self.assertEqual(len(set(original[0]) ^ set(moved[0])), 4)
        self.assertEqual(prototype.trace(simple_model(), MOVED, KINGS)["logical_delta_adds"], 6*256)

    def test_inventory_missing_king_and_extra_king_rejected(self):
        for sfen in (KINGS.replace("4K4", "9"), KINGS.replace("4K4", "3KK4"), KINGS.replace(" - ", " 19P ")):
            with self.subTest(sfen=sfen), self.assertRaises(ValueError):
                prototype.feature_views(sfen, "halfka-kings")

    def test_forward_golden_uses_integer_rounding_and_output_truncation(self):
        model = prototype.load(self.save(), "halfka-kings")
        row = prototype.trace(model, KINGS)
        self.assertEqual((row["acc_us"][0], row["l1"][0], row["l2"][0], row["raw_output"], row["score"]), (3,3,3,49,2))
        base = simple_model("halfkp")
        row = prototype.trace(prototype.load(self.save(base), "halfkp"), KINGS)
        self.assertEqual((row["raw_output"], row["score"]), (-23,0))

    def test_wrap_hidden_thresholds_and_output_clamping(self):
        model = simple_model()
        model["bias"][0] = 32765
        row = prototype.trace(prototype.load(self.save(model), "halfka-kings"), KINGS)
        self.assertEqual(row["acc_us"][0], -32768)
        self.assertEqual(row["l1"][0], 0)
        model = simple_model("halfkp")
        for bias, value in ((-1,0), (63,0), (64,1), (8128,127), (8192,127)):
            model["layers"][0]["bias"][0] = bias
            row = prototype.trace(prototype.load(self.save(model), "halfkp"), KINGS)
            self.assertEqual(row["l1"][0], value)
        for value, expected in ((-25,-1),(25,1),(1000000,30000),(-1000000,-30000)):
            model = simple_model("halfkp")
            model["output_bias"] = value
            self.assertEqual(prototype.trace(prototype.load(self.save(model), "halfkp"), KINGS)["score"], expected)

    def test_header_width_scale_kind_version_and_length_fail_closed(self):
        good = prototype.encode(simple_model())
        changes = [(0, None), (8, 2), (12, 2), (16, 125388), (20,128), (24,64), (28,64), (32,0), (32,129), (36,4097)]
        for offset, value in changes:
            data = bytearray(good)
            if value is None:
                data[0] = 0
            else:
                struct.pack_into("<I", data, offset, value)
            with self.subTest(offset=offset,value=value), self.assertRaises(ValueError):
                prototype.load(self.save(raw=data), "halfka-kings")
        for data in (good[:-1], good+b"x", b"SEKIRW02"):
            with self.subTest(length=len(data)), self.assertRaises(ValueError):
                prototype.load(self.save(raw=data), "halfka-kings")
        with self.assertRaises(ValueError):
            prototype.load(self.save(), "halfkp")

    def test_sparse_order_range_duplicate_and_bias_headroom_rejected(self):
        good = prototype.encode(simple_model())
        for offset, value, code in ((552+516,132560,"I"), (552,138510,"I"), (552,132634,"I"),
                                    (552+2*516,2**31-1,"i"), (len(good)-36,-2**31,"i")):
            data = bytearray(good)
            struct.pack_into("<"+code,data,offset,value)
            with self.subTest(offset=offset), self.assertRaises(ValueError):
                prototype.load(self.save(raw=data), "halfka-kings")

    def test_custom_image_is_rejected_by_current_halfkp_oracle(self):
        with self.assertRaisesRegex(ValueError, "unsupported HalfKP"):
            HalfKpIntegerOracle(self.save(), 24)

    def test_zero_extension_changes_only_feature_count_and_cost(self):
        models = prototype.synthetic_models(prototype.FIXTURES)
        for sfen in prototype.FIXTURES:
            base, zero = (prototype.trace(models[name], sfen) for name in ("baseline", "zero_extension"))
            for key in ("acc_us", "acc_them", "l1", "l2", "raw_output", "score"):
                self.assertEqual(base[key], zero[key])
            self.assertEqual(zero["logical_refresh_adds"]-base["logical_refresh_adds"], 1024)
            self.assertEqual(base["logical_dense_macs"], zero["logical_dense_macs"])
        self.assertEqual(models["baseline"]["layers"], models["king_extension"]["layers"])
        self.assertEqual(models["baseline"]["bias"], models["king_extension"]["bias"])
        self.assertEqual(models["baseline"]["output_weights"], models["king_extension"]["output_weights"])
        for index, row in models["baseline"]["rows"].items():
            self.assertEqual(row, models["king_extension"]["rows"][index])

    def test_move_fixtures_cover_both_colors_and_restore_initial_state(self):
        coverage = {name: 0 for name in ("king_move", "drop", "capture", "promotion")}
        edges = 0
        promoted_by_side = set()
        for _, initial, moves in prototype.MOVE_CASES:
            events = prototype.move_events(initial, moves)
            self.assertEqual(len(events), 1+2*len(moves))
            self.assertEqual(events[0]["sfen"], events[-1]["sfen"])
            for index, event in enumerate(events):
                if event["phase"] == "do":
                    edges += 1
                    for kind in coverage:
                        coverage[kind] += int(event[kind])
                    if event["promotion"]:
                        promoted_by_side.add(events[index-1]["sfen"].split()[1])
        self.assertEqual(edges, 22)
        self.assertEqual(coverage, {"king_move": 11, "drop": 4, "capture": 4, "promotion": 2})
        self.assertEqual(promoted_by_side, {"b", "w"})


@unittest.skipUnless(NATIVE, "set SEKIREI_HALFKA_PROBE for native synthetic inference tests")
class NativePrototypeTest(unittest.TestCase):
    setUp = PrototypeContractTest.setUp
    save = PrototypeContractTest.save

    def native(self, sfens, kind="halfka-kings"):
        return subprocess.run([str(Path(NATIVE).resolve()), str(self.path), kind], input="\n".join(sfens)+"\n",
                              text=True, capture_output=True, timeout=10, check=False)

    def test_native_boundary_arithmetic_matches_independent_reference(self):
        variants = []
        base = simple_model()
        variants.append(base)
        wrap = copy.deepcopy(base)
        wrap["bias"][0] = 32765
        variants.append(wrap)
        for bias in (-23, -25, 25, 1000000, -1000000):
            model = simple_model("halfkp")
            model["output_bias"] = bias
            variants.append(model)
        for scale in (1,16,24,128):
            for model in variants:
                model["fv_scale"] = scale
                parsed = prototype.load(self.save(model), model["kind"])
                completed = self.native([KINGS, MOVED, KINGS], model["kind"])
                self.assertEqual(completed.returncode, 0, completed.stderr)
                rows = [json.loads(line) for line in completed.stdout.splitlines()]
                self.assertEqual(rows, [prototype.trace(parsed,sfen, [KINGS,MOVED,KINGS][index-1] if index else None)
                                        for index,sfen in enumerate((KINGS,MOVED,KINGS))])

    def test_native_format_rejection_and_incompatible_architecture(self):
        good = prototype.encode(simple_model())
        malformed = [good[:-1], good+b"x", b"SEKIRW02"]
        for offset,value in ((8,2),(12,2),(16,1),(20,128),(32,0),(32,129),(36,4097),(552+516,132560),(552,138510)):
            data = bytearray(good)
            struct.pack_into("<I",data,offset,value)
            malformed.append(data)
        for offset, value in ((552+2*516, 2**31-1), (len(good)-36, -2**31)):
            data = bytearray(good)
            struct.pack_into("<i", data, offset, value)
            malformed.append(data)
        for data in malformed:
            self.save(raw=data)
            self.assertEqual(self.native([]).returncode, 2)
        self.save()
        self.assertEqual(self.native([], "halfkp").returncode, 2)
        with self.path.open("wb") as stream:
            stream.truncate(prototype.MAX_BYTES+1)
        self.assertEqual(self.native([]).returncode, 2)

    def test_native_rejects_invalid_piece_inventory(self):
        self.save()
        for sfen in (KINGS.replace("4K4", "9"), KINGS.replace("4K4", "3KK4"), KINGS.replace(" - ", " 19P ")):
            with self.subTest(sfen=sfen):
                self.assertEqual(self.native([sfen]).returncode, 2)

    def native_moves(self, initial, moves, kind):
        return subprocess.run([str(Path(NATIVE).resolve()), str(self.path), kind, "--moves"],
                              input=initial+"\t"+" ".join(moves)+"\n", text=True,
                              capture_output=True, timeout=10, check=False)

    def test_actual_legal_board_moves_and_lifo_undo_match_every_layer(self):
        sfens = [event["sfen"] for _,initial,moves in prototype.MOVE_CASES for event in prototype.move_events(initial,moves)]
        models = prototype.synthetic_models(sfens, 20261108)
        for label, model in models.items():
            parsed = prototype.load(self.save(model), model["kind"])
            for case, initial, moves in prototype.MOVE_CASES:
                with self.subTest(label=label,case=case):
                    result = self.native_moves(initial, moves, model["kind"])
                    self.assertEqual(result.returncode, 0, result.stderr)
                    rows = [json.loads(line) for line in result.stdout.splitlines()]
                    prototype.verify_move_trace(parsed, initial, moves, rows)
                    # Opponent king motion must add/remove two king columns;
                    # this is the missing path in the production HalfKP feature set.
                    for row in rows:
                        if row["phase"] == "do" and row["king_move"]:
                            expected_delta = (512 if model["kind"] == "halfka-kings" else 0) + (512 if row["capture"] else 0)
                            self.assertEqual(row["logical_edge_delta_adds"], expected_delta)
                        if row["phase"] == "undo":
                            self.assertEqual(row["logical_edge_refresh_adds"], 0)

    def test_board_move_inverse_survives_i16_wrap_and_promoted_capture(self):
        sfens = [event["sfen"] for _,initial,moves in prototype.MOVE_CASES for event in prototype.move_events(initial,moves)]
        model = prototype.synthetic_models(sfens, 20261108)["king_extension"]
        model["bias"] = [32767]*256
        for row in model["rows"].values():
            row[:] = [32767]*256
        parsed = prototype.load(self.save(model), model["kind"])
        saw_negative = False
        for _, initial, moves in prototype.MOVE_CASES:
            result = self.native_moves(initial, moves, model["kind"])
            self.assertEqual(result.returncode, 0, result.stderr)
            rows = [json.loads(line) for line in result.stdout.splitlines()]
            prototype.verify_move_trace(parsed, initial, moves, rows)
            saw_negative |= any(value < 0 for row in rows for value in row["trace"]["acc_us"])
        self.assertTrue(saw_negative)

    def test_illegal_board_moves_fail_before_any_move_event(self):
        self.save()
        for move in ("P*7f", "5i5f", "5a5b", '5i5h"suffix'):
            with self.subTest(move=move):
                result = self.native_moves(KINGS, [move], "halfka-kings")
                self.assertEqual(result.returncode, 2)
                rows = [json.loads(line) for line in result.stdout.splitlines()]
                self.assertEqual(len(rows), 1)
                self.assertEqual(rows[0]["phase"], "initial")


if __name__ == "__main__":
    unittest.main()
