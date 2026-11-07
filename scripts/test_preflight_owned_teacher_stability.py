#!/usr/bin/env python3
"""Owned synthetic fixtures only: no searches, training or third-party labels."""
import copy
from contextlib import redirect_stderr, redirect_stdout
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import preflight_owned_teacher_stability as preflight

A = "4k4/9/9/9/9/9/9/9/4K4 b - 1"
B = "3k5/9/9/9/9/9/9/9/4K4 b - 1"
C = "2k6/9/9/9/9/9/9/9/4K4 w - 1"


def tsv(sfen, score, game, result=1):
    return f"{sfen}\t{score}\t{result}\t20\t5i5h\t{game}\n"


def fixture(root):
    document = {
        "schema": preflight.SCHEMA,
        "provenance": {"ownership": "sekirei-owned", "synthetic_fixture": True,
                       "teacher": {"source_commit": "1"*40, "binary_sha256": "2"*64,
                                   "evaluator": "material", "search": {
                                       "threads": 1, "spec_top_n": 0, "use_book": False,
                                       "hash_mb": 16, "tt_policy": "clear-per-position",
                                       "qsearch": {"source_commit": "1"*40, "options": {}}}}},
        "scales": {"fv_scale": 24, "target_score_scale": 400, "teacher_mix": 0.75},
        "thresholds": {"cp_swing": 200, "probability_swing": 0.1},
        "profiles": {"shallow": {"requested_depth": 2}, "deep": {"requested_depth": 4}},
    }
    corpora = {"train": tsv(A, 100, "synthetic-train") + tsv(B, -200, "synthetic-train"),
               "validation": tsv(C, 400, "synthetic-validation", -1)}
    rows = {}
    for profile, values in (("shallow", (100, -200, -400)), ("deep", (300, 100, -800))):
        rows[profile] = []
        for sfen, value in zip((A, B, C), values):
            rows[profile].append({"sfen": sfen, "status": "ok", "observation": {
                "depth": document["profiles"][profile]["requested_depth"],
                "score": {"kind": "cp", "value": value},
                "score_perspective": "black" if sfen == C else "side_to_move",
                "score_bound": "exact", "was_timeout_salvaged": False,
                "bestmove": "5i4h" if profile == "deep" and sfen == B else "5i5h"}})
    return document, corpora, rows


def write_fixture(root, document, corpora, rows, *, set_identities=True):
    for name, content in corpora.items():
        path = root / (name + ".tsv")
        path.write_text(content)
        document[name] = {"path": path.name, "sha256": preflight.file_sha256(path)}
    for name, records in rows.items():
        if set_identities:
            identity = preflight.run_identity(document["provenance"]["teacher"], document["scales"]["fv_scale"],
                                              document["profiles"][name]["requested_depth"])
            for row in records:
                row["run_identity_sha256"] = identity
        path = root / (name + ".jsonl")
        path.write_text("".join(json.dumps(row) + "\n" for row in records))
        document["profiles"][name]["observations"] = {"path": path.name, "sha256": preflight.file_sha256(path)}
    path = root / "inputs.json"
    path.write_text(json.dumps(document))
    return path


class TeacherPreflightTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.document, self.corpora, self.rows = fixture(self.root)

    def write(self, **kwargs):
        return write_fixture(self.root, self.document, self.corpora, self.rows, **kwargs)

    def result(self):
        return preflight.preflight(self.write())

    def test_matched_scores_perspective_and_strict_thresholds(self):
        result = self.result()
        train = result["comparisons"]["train"]
        self.assertEqual(train["counts"], {"unique_positions": 2, "eligible_pairs": 2, "excluded_pairs": 0,
                                          "bestmove_disagreement": 1, "score_sign_flip_nonzero": 1,
                                          "cp_swing_above_threshold": 1, "probability_swing_above_threshold": 2})
        self.assertEqual(train["means"]["signed_cp_delta"], 250)
        validation = result["comparisons"]["validation"]
        self.assertEqual(validation["means"]["signed_cp_delta"], 400)
        self.assertAlmostEqual(validation["means"]["absolute_probability_delta"],
                               preflight.sigmoid(2) - preflight.sigmoid(1))
        self.assertFalse(result["training_permitted"])
        self.assertFalse(result["strength_claim"])

    def test_duplicate_corpus_position_is_one_vote_but_duplicate_observation_fails(self):
        self.corpora["train"] += tsv(A.replace(" 1", " 80"), 500, "synthetic-train")
        result = self.result()
        self.assertEqual(result["corpus"]["train"]["duplicates"], 1)
        self.assertEqual(result["comparisons"]["train"]["means"]["absolute_cp_delta"], 250)
        duplicate = copy.deepcopy(self.rows["deep"][0])
        duplicate["sfen"] = A.replace(" 1", " 80")
        self.rows["deep"].append(duplicate)
        with self.assertRaisesRegex(ValueError, "duplicate canonical"):
            self.result()

    def test_game_split_and_position_split_are_both_enforced(self):
        for content, reason in ((tsv(C, 0, "synthetic-train", -1), "game-level"),
                                (tsv(A.replace(" 1", " 12"), 0, "validation"), "canonical SFEN")):
            self.corpora["validation"] = content
            with self.subTest(reason=reason), self.assertRaisesRegex(ValueError, reason):
                self.result()

    def test_coverage_never_silently_drops_failures(self):
        failed = self.rows["deep"].pop()
        with self.assertRaisesRegex(ValueError, "coverage incomplete"):
            self.result()
        failed.update(status="timeout", observation=None)
        self.rows["deep"].append(failed)
        report = self.result()["comparisons"]["validation"]
        self.assertEqual(report["counts"]["excluded_pairs"], 1)
        self.assertEqual(report["pair_coverage"], 0)
        self.assertIsNone(report["means"]["absolute_cp_delta"])
        self.assertEqual(report["status"], "NO_COMPARABLE_EXACT_CP")
        self.assertEqual(report["exclusion_reasons"], {"deep:timeout": 1})

    def test_mate_bounds_salvage_and_incomplete_depth_are_excluded(self):
        mutations = (({"score": {"kind": "mate", "value": -3}}, "mate_score"),
                     ({"score_bound": "lower"}, "lower_bound"),
                     ({"score_bound": "upper"}, "upper_bound"),
                     ({"was_timeout_salvaged": True}, "timeout_salvaged"),
                     ({"depth": 3}, "requested_depth_not_reached"))
        original = copy.deepcopy(self.rows["deep"][0]["observation"])
        for mutation, reason in mutations:
            self.rows["deep"][0]["observation"] = original | mutation
            with self.subTest(reason=reason):
                report = self.result()["comparisons"]["train"]
                self.assertEqual(report["pair_coverage"], 0.5)
                self.assertEqual(report["exclusion_reasons"], {"deep:" + reason: 1})

    def test_white_black_perspective_inverts_bounds_as_well_as_score(self):
        self.rows["deep"][2]["observation"]["score_bound"] = "lower"
        result = self.result()["comparisons"]["validation"]
        self.assertEqual(result["exclusion_reasons"], {"deep:upper_bound": 1})

    def test_larger_than_declared_budget_is_not_accepted_as_the_same_depth(self):
        self.rows["deep"][0]["observation"]["depth"] = 5
        with self.assertRaisesRegex(ValueError, "exceeds declared"):
            self.result()

    def test_each_explicit_failure_status_keeps_the_position_in_denominator(self):
        for status in ("terminal", "engine_error", "incomplete", "timeout"):
            self.rows["deep"][0].update(status=status, observation=None)
            with self.subTest(status=status):
                report = self.result()["comparisons"]["train"]
                self.assertEqual(report["pair_coverage"], 0.5)
                self.assertEqual(report["exclusion_reasons"], {"deep:" + status: 1})

    def test_fv_scale_cannot_change_cp_or_target_probabilities(self):
        before = self.result()
        self.document["scales"]["fv_scale"] = 128
        after = self.result()
        self.assertEqual(before["comparisons"], after["comparisons"])
        self.assertNotEqual(before["run_identities"], after["run_identities"])
        self.document["scales"]["target_score_scale"] = 800
        scaled = self.result()["comparisons"]["train"]
        self.assertEqual(scaled["means"]["absolute_cp_delta"], 250)
        self.assertNotEqual(before["comparisons"]["train"]["means"]["absolute_probability_delta"],
                            scaled["means"]["absolute_probability_delta"])

    def test_target_mix_and_packer_clipping_are_applied_separately(self):
        self.rows["shallow"][0]["observation"]["score"]["value"] = 40000
        self.rows["deep"][0]["observation"]["score"]["value"] = -40000
        self.document["scales"]["target_score_scale"] = 32000
        report = self.result()["comparisons"]["train"]
        expected = (preflight.sigmoid(1) - preflight.sigmoid(-1)
                    + preflight.sigmoid(100/32000) - preflight.sigmoid(-200/32000)) / 2
        self.assertAlmostEqual(report["means"]["absolute_probability_delta"], expected)
        self.assertAlmostEqual(report["means"]["absolute_mixed_target_delta"], 0.75*expected)
        self.assertEqual(report["profiles"]["shallow"]["packed_score_clipped"], 1)
        self.assertEqual(report["maxima"]["absolute_cp_delta"], 80000)

    def test_profile_identity_binds_teacher_weights_options_depth_and_fv(self):
        self.write()
        mutations = (("binary_sha256", "3"*64), ("evaluator", "owned-halfkp"))
        for key, value in mutations:
            saved = copy.deepcopy(self.document)
            self.document["provenance"]["teacher"][key] = value
            if key == "evaluator":
                self.document["provenance"]["teacher"]["weights_sha256"] = "4"*64
            path = self.write(set_identities=False)
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, "identity mismatch"):
                preflight.preflight(path)
            self.document = saved
        self.document["profiles"]["deep"]["requested_depth"] = 5
        with self.assertRaisesRegex(ValueError, "identity mismatch"):
            preflight.preflight(self.write(set_identities=False))

    def test_unknown_position_and_bad_hash_fail_closed(self):
        self.rows["deep"][0]["sfen"] = A.replace("4k4", "1k7")
        with self.assertRaisesRegex(ValueError, "absent from"):
            self.result()
        self.rows["deep"][0]["sfen"] = A
        path = self.write()
        (self.root / "deep.jsonl").write_text("\n")
        with self.assertRaisesRegex(ValueError, "SHA-256 mismatch"):
            preflight.preflight(path)

    def test_input_mutation_during_read_cannot_publish_report(self):
        original = preflight.index_observations
        def mutate(*args):
            original(*args)
            if args[1] == "deep":
                with (self.root / "deep.jsonl").open("a") as stream:
                    stream.write("\n")
        path = self.write()
        with patch.object(preflight, "index_observations", side_effect=mutate):
            with self.assertRaisesRegex(ValueError, "input changed"):
                preflight.preflight(path)

    def test_missing_semantics_and_nonfinite_scores_are_not_inferred(self):
        original = copy.deepcopy(self.rows["deep"][0]["observation"])
        for field in ("score_bound", "score_perspective", "was_timeout_salvaged", "depth"):
            self.rows["deep"][0]["observation"] = copy.deepcopy(original)
            del self.rows["deep"][0]["observation"][field]
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.result()
        for value in (float("nan"), float("inf"), True, 1.5, -(2**31), 2**31):
            self.rows["deep"][0]["observation"] = copy.deepcopy(original)
            self.rows["deep"][0]["observation"]["score"]["value"] = value
            with self.subTest(value=value), self.assertRaises(ValueError):
                self.result()

    def test_external_teacher_ambiguous_search_and_unpinned_scales_rejected(self):
        original = copy.deepcopy(self.document)
        changes = (("evaluator", "external"), ("search", {"threads": 1}))
        for key, value in changes:
            self.document = copy.deepcopy(original)
            self.document["provenance"]["teacher"][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.result()
        self.document = original
        self.document["scales"]["target_score_scale"] = float("nan")
        with self.assertRaises(ValueError):
            self.result()

    def test_cli_atomic_report_cannot_overwrite_inputs_or_prior_evidence(self):
        path = self.write()
        output = self.root / "report.json"
        with redirect_stdout(io.StringIO()):
            self.assertEqual(preflight.main([str(path), "--output", str(output)]), 0)
        before = output.read_bytes()
        for target in (output, path):
            with self.subTest(target=target), redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit) as error:
                    preflight.main([str(path), "--output", str(target)])
                self.assertEqual(error.exception.code, 2)
        self.assertEqual(output.read_bytes(), before)


if __name__ == "__main__":
    unittest.main()
