#!/usr/bin/env python3
"""Unit tests for gate_phase_a2_weight_ab.py's permutation and §2/§3
diversity-gate/stop-rule logic (see docs/gate/phase_a2_launch_readiness.md).

No third-party dependencies (stdlib unittest only), matching this project's
existing test_gate_dashboard.py convention.

Run: python3 scripts/test_gate_phase_a2_weight_ab.py
"""
import importlib.util
import json
import os
import shutil
import tempfile
import tomllib
import unittest

_SCRIPT_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "gate_phase_a2_weight_ab.py")
_spec = importlib.util.spec_from_file_location("gate_phase_a2_weight_ab_under_test", _SCRIPT_PATH)
gw = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(gw)


class PermutationTests(unittest.TestCase):
    """Category: deterministic permutation (docs/gate/phase_a2_launch_readiness.md §1)."""

    def test_matches_the_preregistered_reference_output_for_n8(self):
        # Regression oracle: computed independently from the doc's exact
        # spec (seed 20260726, shift constants 13/7/17, state|1 init, plain
        # modulo) -- a change here means the algorithm itself changed.
        self.assertEqual(gw.deterministic_permutation(8, 20260726), [7, 3, 2, 5, 0, 4, 6, 1])

    def test_n1_and_n0_edge_cases(self):
        self.assertEqual(gw.deterministic_permutation(1, 20260726), [0])
        self.assertEqual(gw.deterministic_permutation(0, 20260726), [])

    def test_is_deterministic_across_calls(self):
        a = gw.deterministic_permutation(500, 20260726)
        b = gw.deterministic_permutation(500, 20260726)
        self.assertEqual(a, b)

    def test_is_a_valid_permutation_not_just_some_list(self):
        order = gw.deterministic_permutation(1700, 20260726)
        self.assertEqual(sorted(order), list(range(1700)))

    def test_different_seed_gives_a_different_order(self):
        a = gw.deterministic_permutation(200, 20260726)
        b = gw.deterministic_permutation(200, 1)
        self.assertNotEqual(a, b)


class PermutationPersistenceTests(unittest.TestCase):
    """Category: fresh-init generates & persists; resume reloads & verifies."""

    def setUp(self):
        self.outdir = tempfile.mkdtemp(prefix="gate_perm_test_")
        self.corpus = os.path.join(self.outdir, "corpus.sfen")
        with open(self.corpus, "w") as f:
            f.write("# header\n")
            for i in range(10):
                f.write(f"sfen{i}\n")

    def tearDown(self):
        shutil.rmtree(self.outdir, ignore_errors=True)

    def test_fresh_init_writes_permutation_order_file(self):
        order, meta = gw.load_or_create_permutation(self.outdir, self.corpus, 10)
        self.assertTrue(os.path.exists(gw.permutation_order_path(self.outdir)))
        self.assertEqual(sorted(order), list(range(10)))
        self.assertEqual(meta["permutation_seed"], gw.PERMUTATION_SEED)

    def test_resume_reloads_persisted_order_rather_than_regenerating(self):
        order1, meta1 = gw.load_or_create_permutation(self.outdir, self.corpus, 10)
        # Tamper with the persisted file the way a bug (not a real reseed)
        # would -- resume must reflect what's on disk, not silently regenerate.
        tampered = list(reversed(order1))
        with open(gw.permutation_order_path(self.outdir), "w") as f:
            json.dump(tampered, f)
        order2, meta2 = gw.load_or_create_permutation(self.outdir, self.corpus, 10)
        self.assertEqual(order2, tampered)
        self.assertNotEqual(meta2["ordered_output_sha256"], meta1["ordered_output_sha256"])

    def test_resume_with_wrong_length_raises(self):
        gw.load_or_create_permutation(self.outdir, self.corpus, 10)
        with self.assertRaises(SystemExit):
            gw.load_or_create_permutation(self.outdir, self.corpus, 11)


class DiversityAndCountersTests(unittest.TestCase):
    """Category: §2 completed_pairs + corpus-spread + operational counters."""

    def setUp(self):
        self.outdir = tempfile.mkdtemp(prefix="gate_diversity_test_")

    def tearDown(self):
        shutil.rmtree(self.outdir, ignore_errors=True)

    def _make_shard(self, shard_id, start_pos, end_pos, records, stdout_extra=""):
        shard = {"shard_id": shard_id, "start_pos": start_pos, "end_pos": end_pos}
        paths = gw.shard_paths(self.outdir, shard_id)
        with open(paths["jsonl"], "w") as f:
            for rec in records:
                f.write(json.dumps(rec) + "\n")
        with open(paths["stdout"], "w") as f:
            f.write(stdout_extra)
        return shard

    def test_a_pair_needs_both_orientations_to_count(self):
        shard = self._make_shard(
            0, 0, 2,
            [
                {"id": "pos0_pair0", "result": "candidate_win"},
                {"id": "pos0_pair0", "result": "baseline_win"},  # complete pair
                {"id": "pos1_pair0", "result": "candidate_win"},  # lone orientation
            ],
        )
        completed_pairs, spread_ok, counters, observed = gw.compute_diversity_and_counters(
            self.outdir, [shard], num_positions=10
        )
        self.assertEqual(completed_pairs, 1)
        self.assertEqual(counters["illegal_moves"], 0)
        self.assertEqual(counters["engine_errors"], 0)
        self.assertTrue(all(observed.values()))

    def test_distinct_shards_do_not_collide_when_shard_positions_is_one(self):
        # shard_positions=1 (the actual burn-in/gate convention): every
        # shard's own jsonl reuses the same local id ("pos0_pair0") since
        # local_pos is always 0 within a 1-position shard -- only start_pos
        # tells them apart. A grouping key that ignores start_pos would
        # collapse every shard into a single pair.
        shard_a = self._make_shard(
            0, 0, 1,
            [{"id": "pos0_pair0", "result": "candidate_win"}, {"id": "pos0_pair0", "result": "baseline_win"}],
        )
        shard_b = self._make_shard(
            1, 1, 2,
            [{"id": "pos0_pair0", "result": "candidate_win"}, {"id": "pos0_pair0", "result": "baseline_win"}],
        )
        completed_pairs, _, _, _ = gw.compute_diversity_and_counters(
            self.outdir, [shard_a, shard_b], num_positions=10
        )
        self.assertEqual(completed_pairs, 2)

    def test_spread_ok_requires_seven_of_ten_deciles(self):
        # 10 positions -> 10 deciles of width 1. Cover positions 0..5 (6 deciles) -> not ok.
        records = []
        for pos in range(6):
            records.append({"id": f"pos{pos}_pair0", "result": "candidate_win"})
            records.append({"id": f"pos{pos}_pair0", "result": "baseline_win"})
        shard = self._make_shard(0, 0, 10, records)
        completed_pairs, spread_ok, _, _ = gw.compute_diversity_and_counters(
            self.outdir, [shard], num_positions=10
        )
        self.assertEqual(completed_pairs, 6)
        self.assertFalse(spread_ok)

        # Cover positions 0..6 (7 deciles) -> ok.
        records.append({"id": "pos6_pair0", "result": "candidate_win"})
        records.append({"id": "pos6_pair0", "result": "baseline_win"})
        shard2 = self._make_shard(1, 0, 10, records)
        completed_pairs2, spread_ok2, _, _ = gw.compute_diversity_and_counters(
            self.outdir, [shard2], num_positions=10
        )
        self.assertEqual(completed_pairs2, 7)
        self.assertTrue(spread_ok2)

    def test_illegal_engine_error_and_time_forfeit_tags_are_counted_from_stdout(self):
        shard = self._make_shard(
            0, 0, 2,
            [
                {"id": "pos0_pair0", "result": "candidate_win"},
                {"id": "pos0_pair0", "result": "baseline_win"},
            ],
            stdout_extra=(
                "Game    1: A (Black) vs B (White) -> Engine1 Win (illegal)  (10 moves)\n"
                "Game    2: A (Black) vs B (White) -> Engine2 Win (engine error)  (5 moves)\n"
                "Game    3: A (Black) vs B (White) -> Engine2 Win (time forfeit)  (30 moves)\n"
            ),
        )
        _, _, counters, _ = gw.compute_diversity_and_counters(self.outdir, [shard], num_positions=10)
        self.assertEqual(counters["illegal_moves"], 1)
        self.assertEqual(counters["engine_errors"], 1)
        self.assertEqual(counters["time_forfeits"], 1)

    def test_structural_zero_counters_are_always_zero(self):
        shard = self._make_shard(0, 0, 2, [])
        _, _, counters, observed = gw.compute_diversity_and_counters(
            self.outdir, [shard], num_positions=10
        )
        self.assertEqual(counters["protocol_errors"], 0)
        self.assertEqual(counters["material_fallbacks"], 0)
        self.assertEqual(counters["time_forfeits"], 0)
        self.assertTrue(all(observed.values()))


class StopRuleTests(unittest.TestCase):
    """Category: §3 stop rule branch coverage."""

    CLEAN = {
        "illegal_moves": 0, "engine_errors": 0, "weight_load_failures": 0,
        "protocol_errors": 0, "material_fallbacks": 0, "time_forfeits": 0,
    }
    ALL_OBSERVED = {k: True for k in CLEAN}

    def test_pass_boundary_with_enough_pairs_and_spread_finalizes_pass(self):
        verdict, detail = gw.decide_verdict(
            "PASS (elo_diff=177)", 300, True, self.CLEAN, self.ALL_OBSERVED
        )
        self.assertEqual(verdict, "PASS")
        self.assertIsNone(detail)

    def test_fail_boundary_with_enough_pairs_and_spread_finalizes_fail(self):
        verdict, detail = gw.decide_verdict(
            "FAIL (elo_diff=-50)", 300, True, self.CLEAN, self.ALL_OBSERVED
        )
        self.assertEqual(verdict, "FAIL")

    def test_pass_boundary_crossed_but_too_few_pairs_keeps_going(self):
        verdict, detail = gw.decide_verdict(
            "PASS (elo_diff=177)", 299, True, self.CLEAN, self.ALL_OBSERVED
        )
        self.assertIsNone(verdict)

    def test_pass_boundary_crossed_but_spread_not_ok_keeps_going(self):
        verdict, detail = gw.decide_verdict(
            "PASS (elo_diff=177)", 300, False, self.CLEAN, self.ALL_OBSERVED
        )
        self.assertIsNone(verdict)

    def test_no_boundary_crossed_keeps_going(self):
        verdict, detail = gw.decide_verdict(
            "INCONCLUSIVE so far", 300, True, self.CLEAN, self.ALL_OBSERVED
        )
        self.assertIsNone(verdict)

    def test_any_nonzero_counter_contaminates_even_with_a_clean_pass_boundary(self):
        dirty = dict(self.CLEAN, illegal_moves=1)
        verdict, detail = gw.decide_verdict(
            "PASS (elo_diff=177)", 300, True, dirty, self.ALL_OBSERVED
        )
        self.assertEqual(verdict, "CONTAMINATED")
        self.assertEqual(detail, {"illegal_moves": 1})

    def test_contamination_takes_priority_over_a_fail_boundary_too(self):
        dirty = dict(self.CLEAN, weight_load_failures=2)
        verdict, detail = gw.decide_verdict(
            "FAIL (elo_diff=-50)", 300, True, dirty, self.ALL_OBSERVED
        )
        self.assertEqual(verdict, "CONTAMINATED")
        self.assertEqual(detail, {"weight_load_failures": 2})

    def test_unobserved_counter_returns_not_ready_even_with_a_clean_pass_boundary(self):
        unobserved = dict(self.ALL_OBSERVED, time_forfeits=False)
        verdict, detail = gw.decide_verdict(
            "PASS (elo_diff=177)", 300, True, self.CLEAN, unobserved
        )
        self.assertEqual(verdict, "NOT_READY")
        self.assertEqual(detail, {"unobserved_counters": ["time_forfeits"]})

    def test_not_ready_takes_priority_over_contamination_too(self):
        # An unobservable counter means we can't even trust "clean" here --
        # NOT_READY must win regardless of what the (possibly meaningless)
        # counter values happen to show.
        unobserved = dict(self.ALL_OBSERVED, time_forfeits=False)
        dirty = dict(self.CLEAN, illegal_moves=1)
        verdict, detail = gw.decide_verdict(
            "PASS (elo_diff=177)", 300, True, dirty, unobserved
        )
        self.assertEqual(verdict, "NOT_READY")


class ManifestTests(unittest.TestCase):
    """Category: docs/gate/phase_a2_manifest_contract.md's TOML writer -- fixture round-trips."""

    def setUp(self):
        self.outdir = tempfile.mkdtemp(prefix="gate_manifest_test_")

    def tearDown(self):
        shutil.rmtree(self.outdir, ignore_errors=True)

    def test_immutable_section_roundtrips_through_tomllib(self):
        fields = {
            "run_id": "b1_vs_a_run2_20260803",
            "candidate_weight_sha256": "a" * 64,
            "permutation_seed": 20260726,
            "byoyomi_ms": 1500,
            "alpha": 0.05,
            "speculation": False,
            "fresh_process_policy": 'one process per shard, quotes " and \\ backslash',
        }
        gw.write_manifest_immutable(self.outdir, fields)
        with open(gw.manifest_path(self.outdir), "rb") as f:
            manifest = tomllib.load(f)
        self.assertEqual(manifest["schema_version"], 1)
        for k, v in fields.items():
            self.assertEqual(manifest["immutable"][k], v)

    def test_progress_appends_accumulate_as_array_of_tables_not_overwritten(self):
        gw.write_manifest_immutable(self.outdir, {"run_id": "r"})
        gw.append_manifest_progress(self.outdir, {"status": "running", "completed_games": 10})
        gw.append_manifest_progress(self.outdir, {"status": "decisive", "completed_games": 166})
        with open(gw.manifest_path(self.outdir), "rb") as f:
            manifest = tomllib.load(f)
        self.assertEqual(len(manifest["progress"]), 2)
        self.assertEqual(manifest["progress"][0]["completed_games"], 10)
        self.assertEqual(manifest["progress"][1]["completed_games"], 166)
        self.assertEqual(manifest["progress"][1]["status"], "decisive")

    def test_sprt_llr_bounds_match_the_real_burnins_observed_figure(self):
        # results/phase_a2/b1_vs_a_burnin's actual SPRT output recorded
        # bounds [-2.944, 2.944] at alpha=beta=0.05 -- cross-check against
        # real observed data, not just the formula in isolation.
        lower, upper = gw.sprt_llr_bounds(0.05, 0.05)
        self.assertAlmostEqual(upper, 2.944, places=3)
        self.assertAlmostEqual(lower, -2.944, places=3)


if __name__ == "__main__":
    unittest.main(verbosity=2)
