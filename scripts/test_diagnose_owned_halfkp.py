#!/usr/bin/env python3
"""Synthetic diagnostics only: no real games, trainer updates, or engine runs."""
import copy
from contextlib import closing
import importlib.util
import json
import math
from pathlib import Path
import sqlite3
import struct
import sys
import tempfile
import unittest
from unittest.mock import patch
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parent))
import diagnose_owned_halfkp as diagnostic

HAS_ARRAYS = all(importlib.util.find_spec(name) is not None for name in ("numpy", "torch"))


class DataDiagnosticsTest(unittest.TestCase):
    A = "4k4/9/9/9/9/9/9/9/4K4 b - 1"
    B = "3k5/9/9/9/9/9/9/9/4K4 b - 1"
    C = "2k6/9/9/9/9/9/9/9/4K4 w - 1"

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    @staticmethod
    def row(sfen, score, game, result=1, ply=20):
        return f"{sfen}\t{score}\t{result}\t{ply}\t5i5h\t{game}\n"

    def inputs(self, train=None, validation=None, scale=400, fv=24, mix=1):
        train = train if train is not None else self.row(self.A, 0, "train") + self.row(self.B, 800, "train")
        validation = validation if validation is not None else self.row(self.C, -400, "validation", -1)
        document = {
            "schema": diagnostic.SCHEMA,
            "provenance": {"ownership": "sekirei-owned", "synthetic_fixture": True,
                           "teacher": {"source_commit": "1"*40, "binary_sha256": "2"*64,
                                       "evaluator": "material", "search": {"label_depth": 4, "qsearch_revision": "1"*40}}},
            "scales": {"fv_scale": fv, "target_score_scale": scale, "teacher_mix": mix},
        }
        for arm, content in (("train", train), ("validation", validation)):
            path = self.root / (arm + ".tsv")
            path.write_text(content)
            document[arm] = {"path": path.name, "sha256": diagnostic.file_sha256(path)}
        manifest = self.root / "inputs.json"
        manifest.write_text(json.dumps(document))
        return manifest, document

    def test_duplicates_get_one_vote_and_move_counters_do_not_make_new_positions(self):
        train = (self.row(self.A, 0, "train") + self.row(self.A.replace(" 1", " 99"), 0, "train")
                 + self.row(self.B, 800, "train"))
        path, _ = self.inputs(train=train)
        result = diagnostic.diagnose(path)
        arm = result["data"]["train"]
        self.assertEqual((arm["raw_rows"], arm["unique_positions"], arm["duplicates"], arm["games"]), (3,2,1,1))
        self.assertEqual(arm["unique_position_mean_cp"], 400)
        self.assertAlmostEqual(arm["unique_position_mean_target"], (0.5 + diagnostic.sigmoid(2))/2)
        self.assertFalse(result["strength_claim"])

    def test_duplicate_label_disagreement_is_reported_without_selecting_a_label(self):
        path, _ = self.inputs(train=self.row(self.A, 0, "a") + self.row(self.A, 800, "b", result=-1))
        result = diagnostic.diagnose(path)["data"]["train"]
        self.assertEqual(result["positions_with_different_scores"], 1)
        self.assertEqual(result["positions_with_different_results"], 1)
        self.assertAlmostEqual(result["unique_position_mean_teacher_probability"], (0.5+diagnostic.sigmoid(2))/2)
        self.assertEqual(result["unique_position_mean_outcome_target"], 0.5)

    def test_game_overlap_and_canonical_position_overlap_both_fail(self):
        for validation, message in ((self.row(self.C, 0, "train", -1), "game-level"),
                                    (self.row(self.A.replace(" 1", " 999"), 0, "validation"), "canonical SFEN")):
            path, _ = self.inputs(validation=validation)
            with self.subTest(message=message), self.assertRaisesRegex(ValueError, message):
                diagnostic.diagnose(path)

    def test_reordered_hands_still_overlap(self):
        path, _ = self.inputs(train=self.row(self.A.replace(" - ", " PR "), 0, "train"),
                              validation=self.row(self.A.replace(" - ", " RP "), 0, "validation"))
        with self.assertRaisesRegex(ValueError, "canonical SFEN"):
            diagnostic.diagnose(path)

    def test_inconsistent_game_results_and_legacy_rows_fail(self):
        for train, message in ((self.row(self.A, 0, "train") + self.row(self.B, 0, "train", -1), "inconsistent"),
                               ("\t".join(self.row(self.A, 0, "train").split("\t")[:5]) + "\n", "six-column")):
            path, _ = self.inputs(train=train)
            with self.subTest(message=message), self.assertRaisesRegex(ValueError, message):
                diagnostic.diagnose(path)

    def test_target_scale_is_independent_of_network_fv_scale(self):
        path, _ = self.inputs()
        original = diagnostic.diagnose(path)
        path, _ = self.inputs(fv=16)
        other_fv = diagnostic.diagnose(path)
        self.assertEqual(original["data"], other_fv["data"])
        path, _ = self.inputs(scale=800)
        other_target = diagnostic.diagnose(path)
        self.assertNotEqual(original["data"]["train"]["unique_position_mean_target"],
                            other_target["data"]["train"]["unique_position_mean_target"])
        self.assertEqual(other_target["scales"]["fv_scale"], 24)

    def test_packed_label_clamp_and_outcome_mix_follow_training_contract(self):
        path, _ = self.inputs(train=self.row(self.A, 40000, "train"), scale=10000)
        stats = diagnostic.diagnose(path)["data"]["train"]
        self.assertEqual(stats["raw_max_cp"], 40000)
        self.assertEqual(stats["labels_clamped_by_packer"], 1)
        self.assertEqual(stats["unique_position_mean_packed_cp"], 32000)
        self.assertAlmostEqual(stats["unique_position_mean_target"], diagnostic.sigmoid(3.2))
        path, _ = self.inputs(mix=0)
        self.assertEqual(diagnostic.diagnose(path)["data"]["train"]["unique_position_mean_target"], 1)

    def test_bad_hash_external_teacher_and_missing_scales_fail(self):
        path, original = self.inputs()
        variants = []
        changed = copy.deepcopy(original); changed["train"]["sha256"] = "0"*64; variants.append((changed, "SHA-256"))
        changed = copy.deepcopy(original); changed["provenance"]["teacher"]["evaluator"] = "external"; variants.append((changed, "evaluator"))
        changed = copy.deepcopy(original); del changed["scales"]["target_score_scale"]; variants.append((changed, "target_score_scale"))
        changed = copy.deepcopy(original); changed["scales"]["fv_scale"] = True; variants.append((changed, "fv_scale"))
        for document, message in variants:
            path.write_text(json.dumps(document))
            with self.subTest(message=message), self.assertRaisesRegex(ValueError, message):
                diagnostic.diagnose(path)

    def test_source_mutation_during_scan_is_rejected(self):
        path, _ = self.inputs()
        original = diagnostic.index_corpus
        def mutate(*args):
            result = original(*args)
            with (self.root/"train.tsv").open("a") as stream:
                stream.write("\n")
            return result
        with patch.object(diagnostic, "index_corpus", side_effect=mutate), self.assertRaisesRegex(ValueError, "changed during"):
            diagnostic.diagnose(path)

    def test_activation_endpoint_and_strict_saturation_are_separate(self):
        from test_halfkp_integer_oracle import sparse_network
        path, _ = self.inputs()
        bins = {name: self.root/(name+".bin") for name in ("baseline", "candidate")}
        for name, binary in bins.items():
            offsets = sparse_network(binary)
            if name == "candidate":
                with binary.open("r+b") as stream:
                    stream.seek(offsets["l1_bias"])
                    stream.write(struct.pack("<i", 8192+32))
        def pack(argv, **_):
            from verify_halfkp_native_parity import PACKED
            sfens = [line.split("\t")[0] for line in Path(argv[3]).read_text().splitlines()]
            Path(argv[2]).write_bytes(b"".join(PACKED.pack(0 if s.split()[1] == "b" else 1,
                0, 0, 0, 0, *([1548]*76)) for s in sfens))
            return SimpleNamespace(returncode=0, stdout="", stderr="")
        with closing(sqlite3.connect(self.root/"index.sqlite3")) as connection:
            diagnostic.index_corpus(connection, {name: self.root/(name+".tsv") for name in ("train", "validation")}, 400, 1)
            with patch.object(diagnostic.subprocess, "run", side_effect=pack):
                report = diagnostic.activation_diagnostics(connection, bins, Path("unused-pack"), self.root, 24, 1)
        self.assertEqual(report["sample_positions"], 2)
        candidate = report["models"]["candidate"]["train"]
        self.assertEqual(candidate["l1"]["preclip_above_127"], 1)
        self.assertEqual(candidate["l2"]["preclip_above_127"], 0)
        self.assertEqual(candidate["l2"]["at_127"], 1)


@unittest.skipUnless(HAS_ARRAYS, "numpy and torch required")
class QuantizationDiagnosticsTest(unittest.TestCase):
    def test_float_updates_lost_in_export_and_bias_rounding_are_distinct(self):
        import numpy as np
        old, _, _ = diagnostic.quantized_chunk([0.0, 0.0], "ft_weight", 24)
        new, recipe, export = diagnostic.quantized_chunk([0.001, 0.01], "ft_weight", 24)
        self.assertEqual(new.tolist(), [0,1])
        self.assertEqual(int(np.count_nonzero(new != old)), 1)
        bias, _, _ = diagnostic.quantized_chunk([0.5/127], "l1_bias", 24)
        self.assertEqual(bias.tolist(), [64])
        self.assertEqual((recipe, export), (0,0))

    def test_recipe_clipping_and_file_type_clipping_are_reported_separately(self):
        value, recipe, export = diagnostic.quantized_chunk([8, -8], "ft_weight", 24, factorized=True)
        self.assertEqual(value.tolist(), [762,-762])
        self.assertEqual((recipe,export), (2,0))
        value, recipe, export = diagnostic.quantized_chunk([3,-3], "l1_weight", 24)
        self.assertEqual(value.tolist(), [127,-127])
        self.assertEqual((recipe,export), (2,2))
        with self.assertRaisesRegex(ValueError, "nonfinite"):
            diagnostic.quantized_chunk([math.nan], "ft_weight", 24)

    def test_float_update_reader_binds_quantization_to_actual_section_bytes(self):
        import torch
        torch.set_num_threads(1)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            baseline = torch.zeros((1,32))
            candidate = baseline.clone(); candidate[0,0] = 0.001; candidate[0,1] = 127/24
            snapshots, bins = [], []
            for name, tensor, encoded in (("before", baseline, bytes(32)), ("after", candidate, bytes([0,1])+bytes(30))):
                snapshot, binary = root/(name+".pt"), root/(name+".bin")
                torch.save({"out.weight": tensor}, snapshot)
                binary.write_bytes(encoded)
                snapshots.append(snapshot); bins.append(binary)
            # The full nn.bin parser is tested independently. This focused
            # fixture verifies float checkpoint reading against a real byte
            # section without allocating two full-size FT tables.
            section = {"name": "out_weight", "total": 32, "code": "b", "offset": 0}
            with patch.object(diagnostic, "inspect_file", return_value={"sections": [section]}):
                result = diagnostic.float_parameter_diagnostics(snapshots, bins, 24)
                stats = result["sections"][0]
                self.assertEqual((stats["float_changed"], stats["integer_changed"],
                                  stats["float_changed_but_integer_unchanged"]), (2,1,1))
                bins[1].write_bytes(bytes(32))
                with self.assertRaisesRegex(ValueError, "does not reproduce"):
                    diagnostic.float_parameter_diagnostics(snapshots, bins, 24)


if __name__ == "__main__":
    unittest.main()
