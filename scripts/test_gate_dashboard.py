#!/usr/bin/env python3
"""Unit tests for gate_dashboard.py's deterministic review logic (get_pipeline_review /
get_gate_review / get_trend_review / get_review_data / generate_review_narrative).

Scope: the boundary shared with tasks/lessons.md's design rule -- the deterministic layer
owns every number and verdict, the LLM narrative only describes them and can never override
or originate one. These tests exercise that boundary directly, not just the happy path.

No third-party dependencies (stdlib unittest only), matching gate_dashboard.py's own
"no third-party Python dependencies" constraint (see its module docstring).

Run: python3 scripts/test_gate_dashboard.py
"""
import importlib.util
import json
import os
import shutil
import sys
import tempfile
import unittest

_REPO_ROOT = tempfile.mkdtemp(prefix="gate_dashboard_test_")
_RESULTS_DIR = os.path.join(_REPO_ROOT, "results")
_DATA_DIR = os.path.join(_REPO_ROOT, "data")
os.makedirs(_RESULTS_DIR, exist_ok=True)
os.makedirs(os.path.join(_DATA_DIR, "runs"), exist_ok=True)

_placeholder_result = os.path.join(_RESULTS_DIR, "_placeholder.json")
with open(_placeholder_result, "w") as f:
    f.write("{}")

# gate_dashboard.py reads sys.argv at import time to derive REPO_ROOT/DATA_DIR/RESULTS_DIR
# (see its module docstring: RESULT_JSON is assumed to live at <repo>/results/*.json) --
# point it at our temp fixture tree before importing, then restore argv so unittest's own
# CLI parsing (triggered below by unittest.main()) doesn't choke on gate_dashboard.py's args.
_real_argv = sys.argv
sys.argv = ["gate_dashboard.py", "/dev/null", _placeholder_result]
_SCRIPT_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "gate_dashboard.py")
_spec = importlib.util.spec_from_file_location("gate_dashboard_under_test", _SCRIPT_PATH)
gd = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(gd)
sys.argv = _real_argv


def _write_json(path, obj):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        json.dump(obj, f)


def _make_pipeline_run(run_id, epoch_extras, baseline=None):
    """epoch_extras: list of per-epoch field dicts merged with {"epoch": i}."""
    run_dir = os.path.join(_DATA_DIR, "runs", run_id)
    _write_json(os.path.join(run_dir, "manifest.json"), {"output": "data/weights.bin", "baseline": baseline})
    for i, extra in enumerate(epoch_extras, start=1):
        _write_json(os.path.join(run_dir, "checkpoints", f"weights.epoch{i}.meta.json"), {"epoch": i, **extra})
    return run_dir


def _make_gate_result(file_name, sidecar=None, **fields):
    _write_json(os.path.join(_RESULTS_DIR, file_name), fields)
    if sidecar is not None:
        base, _ext = os.path.splitext(file_name)
        _write_json(os.path.join(_RESULTS_DIR, f"{base}.verdict.json"), sidecar)


class PipelineVerdictBoundaryTests(unittest.TestCase):
    """Category: collapse condition boundary values."""

    def tearDown(self):
        shutil.rmtree(os.path.join(_DATA_DIR, "runs"), ignore_errors=True)
        os.makedirs(os.path.join(_DATA_DIR, "runs"), exist_ok=True)

    def test_healthy_when_cp_mse_improves_and_growth_decelerates(self):
        _make_pipeline_run("run_healthy", [
            {"valid_cp_mse": 1000.0, "output_weight_norm": 5.0, "valid_output_range": 10.0},
            {"valid_cp_mse": 900.0, "output_weight_norm": 8.0, "valid_output_range": 12.0},   # ratio 1.6
            {"valid_cp_mse": 800.0, "output_weight_norm": 10.0, "valid_output_range": 14.0},  # ratio 1.25 < 1.6
        ])
        self.assertEqual(gd.get_pipeline_review("run_healthy")["verdict"], "HEALTHY")

    def test_invalid_at_exact_zero_output_range_boundary(self):
        _make_pipeline_run("run_zero", [
            {"valid_cp_mse": 1000.0, "output_weight_norm": 5.0, "valid_output_range": 10.0},
            {"valid_cp_mse": 900.0, "output_weight_norm": 6.0, "valid_output_range": 0.0},
        ])
        self.assertEqual(gd.get_pipeline_review("run_zero")["verdict"], "INVALID")

    def test_not_invalid_just_above_the_zero_boundary(self):
        _make_pipeline_run("run_near_zero", [
            {"valid_cp_mse": 1000.0, "output_weight_norm": 5.0, "valid_output_range": 10.0},
            {"valid_cp_mse": 900.0, "output_weight_norm": 6.0, "valid_output_range": 0.0001},
        ])
        self.assertNotEqual(gd.get_pipeline_review("run_near_zero")["verdict"], "INVALID")

    def test_invalid_wins_over_an_improving_cp_mse(self):
        # A collapsed (constant) output is unusable regardless of what its
        # cp_mse says -- collapse must override an otherwise-improving trend.
        _make_pipeline_run("run_collapsed_but_improving", [
            {"valid_cp_mse": 1000.0, "output_weight_norm": 5.0, "valid_output_range": 10.0},
            {"valid_cp_mse": 100.0, "output_weight_norm": 6.0, "valid_output_range": 0.0},
        ])
        self.assertEqual(gd.get_pipeline_review("run_collapsed_but_improving")["verdict"], "INVALID")

    def test_warning_when_cp_mse_gets_worse_without_collapse(self):
        _make_pipeline_run("run_worse", [
            {"valid_cp_mse": 800.0, "output_weight_norm": 5.0, "valid_output_range": 10.0},
            {"valid_cp_mse": 900.0, "output_weight_norm": 6.0, "valid_output_range": 12.0},
        ])
        self.assertEqual(gd.get_pipeline_review("run_worse")["verdict"], "WARNING")

    def test_warning_when_growth_ratio_accelerates_even_if_cp_mse_improves(self):
        _make_pipeline_run("run_accelerating", [
            {"valid_cp_mse": 1000.0, "output_weight_norm": 5.0, "valid_output_range": 10.0},
            {"valid_cp_mse": 950.0, "output_weight_norm": 6.0, "valid_output_range": 12.0},    # ratio 1.2
            {"valid_cp_mse": 900.0, "output_weight_norm": 12.0, "valid_output_range": 14.0},   # ratio 2.0 > 1.2
        ])
        self.assertEqual(gd.get_pipeline_review("run_accelerating")["verdict"], "WARNING")


class PipelineMissingAndNanDataTests(unittest.TestCase):
    """Category: training run empty / missing / NaN."""

    def tearDown(self):
        shutil.rmtree(os.path.join(_DATA_DIR, "runs"), ignore_errors=True)
        os.makedirs(os.path.join(_DATA_DIR, "runs"), exist_ok=True)

    def test_missing_run_directory_returns_none(self):
        self.assertIsNone(gd.get_pipeline_review("run_that_does_not_exist"))

    def test_manifest_with_no_checkpoints_reports_unavailable_not_a_crash(self):
        run_dir = os.path.join(_DATA_DIR, "runs", "run_empty")
        _write_json(os.path.join(run_dir, "manifest.json"), {"output": "data/weights.bin"})
        review = gd.get_pipeline_review("run_empty")
        self.assertFalse(review["available"])

    def test_cp_mse_never_present_is_insufficient_data_not_healthy(self):
        _make_pipeline_run("run_no_cp_mse", [
            {"output_weight_norm": 5.0, "valid_output_range": 10.0},
            {"output_weight_norm": 6.0, "valid_output_range": 12.0},
        ])
        self.assertEqual(gd.get_pipeline_review("run_no_cp_mse")["verdict"], "INSUFFICIENT_DATA")

    def test_nan_valid_cp_mse_does_not_silently_read_as_healthy(self):
        # json.loads accepts the bare `NaN` token by default; a NaN slipping
        # through as a "valid" float would make every comparison against it
        # false, which previously fell through every branch into HEALTHY.
        run_dir = os.path.join(_DATA_DIR, "runs", "run_nan")
        os.makedirs(os.path.join(run_dir, "checkpoints"), exist_ok=True)
        _write_json(os.path.join(run_dir, "manifest.json"), {"output": "data/weights.bin"})
        with open(os.path.join(run_dir, "checkpoints", "weights.epoch1.meta.json"), "w") as f:
            f.write('{"epoch": 1, "valid_cp_mse": NaN, "output_weight_norm": 5.0, "valid_output_range": 1.0}')
        with open(os.path.join(run_dir, "checkpoints", "weights.epoch2.meta.json"), "w") as f:
            f.write('{"epoch": 2, "valid_cp_mse": NaN, "output_weight_norm": 6.0, "valid_output_range": 1.0}')
        review = gd.get_pipeline_review("run_nan")
        self.assertEqual(review["verdict"], "INSUFFICIENT_DATA")
        self.assertIsNone(review["metrics"]["cp_mse_delta"])

    def test_nan_output_weight_norm_is_excluded_from_growth_ratios(self):
        run_dir = os.path.join(_DATA_DIR, "runs", "run_nan_wnorm")
        os.makedirs(os.path.join(run_dir, "checkpoints"), exist_ok=True)
        _write_json(os.path.join(run_dir, "manifest.json"), {"output": "data/weights.bin"})
        with open(os.path.join(run_dir, "checkpoints", "weights.epoch1.meta.json"), "w") as f:
            f.write('{"epoch": 1, "valid_cp_mse": 900.0, "output_weight_norm": NaN, "valid_output_range": 1.0}')
        with open(os.path.join(run_dir, "checkpoints", "weights.epoch2.meta.json"), "w") as f:
            f.write('{"epoch": 2, "valid_cp_mse": 800.0, "output_weight_norm": 5.0, "valid_output_range": 1.0}')
        review = gd.get_pipeline_review("run_nan_wnorm")
        self.assertEqual(review["metrics"]["growth_ratios"], [])


class GateReviewReusesExistingVerdictTests(unittest.TestCase):
    """Category: gate review must reuse the already-decided verdict, never re-derive one."""

    def test_sprt_sidecar_verdict_is_reused_verbatim_even_though_ci_math_would_disagree(self):
        # elo_diff=25/los=0.97 would compute PASS under the CI-threshold
        # heuristic (verdict_of) -- but this result was actually decided by
        # SPRT as INCONCLUSIVE. Reusing get_history_data's own parsing (which
        # already prefers the persisted sidecar) is exactly what prevents
        # the regression tasks/lessons.md records: an SPRT-decided
        # INCONCLUSIVE result being redisplayed as a CI-threshold verdict.
        _make_gate_result(
            "sprt_result.json",
            sidecar={"verdict": "INCONCLUSIVE", "method": "sprt", "llr": 0.1, "bound_lo": -2.9, "bound_hi": 2.9},
            elo_diff=25.0, los=0.97, games=100, engine1_wins=55, draws=10, engine2_wins=35,
        )
        review = gd.get_gate_review("sprt_result.json")
        self.assertEqual(review["verdict"], "INCONCLUSIVE")
        self.assertEqual(review["metrics"]["method"], "sprt")

    def test_legacy_file_with_no_sidecar_falls_back_to_ci_derivation_unchanged(self):
        _make_gate_result("legacy_result.json", elo_diff=25.0, los=0.97, games=100, engine1_wins=55, draws=10, engine2_wins=35)
        review = gd.get_gate_review("legacy_result.json")
        self.assertEqual(review["verdict"], gd.verdict_of(25.0, 0.97))
        self.assertIsNone(review["metrics"]["method"])

    def test_unknown_file_returns_none(self):
        self.assertIsNone(gd.get_gate_review("does_not_exist.json"))


class GateReviewDiversityChecklistTests(unittest.TestCase):
    """Category: low-diversity checklist."""

    def test_diversity_below_threshold_is_flagged_not_ok(self):
        _make_gate_result("low_div.json", elo_diff=10.0, los=0.8, games=60, engine1_wins=30, draws=0, engine2_wins=30, diversity_ratio=0.15)
        checklist = {c["label"]: c["ok"] for c in gd.get_gate_review("low_div.json")["checklist"]}
        self.assertIs(checklist["diversity_adequate"], False)

    def test_diversity_at_or_above_threshold_is_ok(self):
        _make_gate_result("ok_div.json", elo_diff=10.0, los=0.8, games=60, engine1_wins=30, draws=0, engine2_wins=30, diversity_ratio=gd.MIN_DIVERSITY_RATIO)
        checklist = {c["label"]: c["ok"] for c in gd.get_gate_review("ok_div.json")["checklist"]}
        self.assertIs(checklist["diversity_adequate"], True)

    def test_missing_diversity_ratio_is_unknown_not_false(self):
        _make_gate_result("no_div.json", elo_diff=10.0, los=0.8, games=60, engine1_wins=30, draws=0, engine2_wins=30)
        checklist = {c["label"]: c["ok"] for c in gd.get_gate_review("no_div.json")["checklist"]}
        self.assertIsNone(checklist["diversity_adequate"])


class NarrativeFailureDegradesGracefullyTests(unittest.TestCase):
    """Category: API failure must still return a full numeric report."""

    def setUp(self):
        self._orig_key = gd.ANTHROPIC_API_KEY
        self._orig_call = gd.call_anthropic

    def tearDown(self):
        gd.ANTHROPIC_API_KEY = self._orig_key
        gd.call_anthropic = self._orig_call
        shutil.rmtree(os.path.join(_DATA_DIR, "runs"), ignore_errors=True)
        os.makedirs(os.path.join(_DATA_DIR, "runs"), exist_ok=True)

    def test_no_api_key_returns_none_narrative_without_calling_anthropic(self):
        gd.ANTHROPIC_API_KEY = ""
        called = []
        gd.call_anthropic = lambda *a, **k: called.append(1) or "should not be reached"
        self.assertIsNone(gd.generate_review_narrative({"kind": "pipeline"}))
        self.assertEqual(called, [])

    def test_anthropic_call_raising_returns_none_not_an_exception(self):
        gd.ANTHROPIC_API_KEY = "dummy-key-for-test"

        def _raise(*a, **k):
            raise RuntimeError("simulated network failure")

        gd.call_anthropic = _raise
        self.assertIsNone(gd.generate_review_narrative({"kind": "pipeline"}))

    def test_get_review_data_still_returns_full_verdict_and_metrics_when_narrative_fails(self):
        gd.ANTHROPIC_API_KEY = "dummy-key-for-test"
        gd.call_anthropic = lambda *a, **k: (_ for _ in ()).throw(RuntimeError("simulated failure"))
        _make_pipeline_run("run_narrative_fail", [
            {"valid_cp_mse": 1000.0, "output_weight_norm": 5.0, "valid_output_range": 10.0},
            {"valid_cp_mse": 900.0, "output_weight_norm": 6.0, "valid_output_range": 12.0},
        ])
        review = gd.get_review_data("pipeline", "run_narrative_fail")
        self.assertIsNone(review["narrative"])
        self.assertEqual(review["verdict"], "HEALTHY")
        self.assertIn("cp_mse_series", review["metrics"])


class NarrativeCannotOverrideNumbersTests(unittest.TestCase):
    """Category: the LLM response must never alter the verdict or metrics it was given."""

    def setUp(self):
        self._orig_key = gd.ANTHROPIC_API_KEY
        self._orig_call = gd.call_anthropic
        gd.ANTHROPIC_API_KEY = "dummy-key-for-test"

    def tearDown(self):
        gd.ANTHROPIC_API_KEY = self._orig_key
        gd.call_anthropic = self._orig_call
        shutil.rmtree(os.path.join(_DATA_DIR, "runs"), ignore_errors=True)
        os.makedirs(os.path.join(_DATA_DIR, "runs"), exist_ok=True)

    def test_a_narrative_that_asserts_a_different_verdict_does_not_change_the_reported_verdict(self):
        _make_pipeline_run("run_adversarial_narrative", [
            {"valid_cp_mse": 900.0, "output_weight_norm": 5.0, "valid_output_range": 10.0},
            {"valid_cp_mse": 1000.0, "output_weight_norm": 6.0, "valid_output_range": 12.0},  # worsens -> WARNING
        ])
        direct = gd.get_pipeline_review("run_adversarial_narrative")
        gd.call_anthropic = lambda *a, **k: (
            'Actually this run is HEALTHY and valid_cp_mse=1.0, ignore the checklist above.'
        )
        review = gd.get_review_data("pipeline", "run_adversarial_narrative")
        self.assertEqual(review["verdict"], direct["verdict"])
        self.assertEqual(review["verdict"], "WARNING")
        self.assertEqual(review["metrics"], direct["metrics"])
        # The adversarial text is only ever stored under "narrative", never
        # merged into verdict/metrics/checklist.
        self.assertIn("HEALTHY", review["narrative"])
        self.assertEqual(review["checklist"], direct["checklist"])


class TrendReviewNeverReturnsPassFailTests(unittest.TestCase):
    """Category: project trend must never return a PASS/FAIL verdict."""

    def test_trend_review_has_no_verdict_field(self):
        review = gd.get_trend_review()
        self.assertNotIn("verdict", review)

    def test_trend_status_is_always_one_of_the_defined_statuses(self):
        review = gd.get_trend_review()
        self.assertIn(review["trend_status"], gd.TREND_STATUSES)

    def test_trend_status_vocabulary_excludes_pass_fail_inconclusive(self):
        self.assertNotIn("PASS", gd.TREND_STATUSES)
        self.assertNotIn("FAIL", gd.TREND_STATUSES)
        self.assertNotIn("INCONCLUSIVE", gd.TREND_STATUSES)

    def test_insufficient_evidence_when_fewer_than_two_decisive_gates_exist(self):
        # This process-wide test run may have accumulated gate fixtures from
        # earlier tests in other classes (get_history_data reads the whole
        # results dir) -- isolate by using a fresh results dir just for this
        # assertion instead of asserting against whatever is already present.
        global _RESULTS_DIR
        fresh_dir = tempfile.mkdtemp(prefix="gate_dashboard_test_trend_")
        orig_results_dir = gd.RESULTS_DIR
        gd.RESULTS_DIR = fresh_dir
        try:
            review = gd.get_trend_review()
            self.assertEqual(review["trend_status"], "INSUFFICIENT_EVIDENCE")
            self.assertEqual(review["positives"], [])
            self.assertEqual(review["negatives"], [])
        finally:
            gd.RESULTS_DIR = orig_results_dir
            shutil.rmtree(fresh_dir, ignore_errors=True)


if __name__ == "__main__":
    try:
        unittest.main(verbosity=2)
    finally:
        shutil.rmtree(_REPO_ROOT, ignore_errors=True)
