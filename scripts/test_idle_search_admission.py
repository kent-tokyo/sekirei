"""Idle admission tests use fake engines, clocks and search calls only."""
import json
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory
from types import SimpleNamespace
from unittest.mock import patch

import paired_search_diagnostics as paired


IDLE = {"available": True, "busy_process_threshold_percent": 25,
        "busy_processes": [], "load_average": [0, 0, 0]}
BUSY = {**IDLE, "busy_processes": [{"pid": 123, "cpu_percent": 25, "command": "other-job"}]}


class Clock:
    def __init__(self):
        self.now = 0
        self.sleeps = []

    def monotonic(self):
        return self.now

    def sleep(self, seconds):
        self.sleeps.append(seconds)
        self.now += seconds


class IdleAdmissionTests(unittest.TestCase):
    def setUp(self):
        self.tmp = TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.engine = self.root / "engine"
        self.engine.write_bytes(b"unexecutable fixture, never launched")
        self.positions = self.root / "positions"
        self.positions.write_text("startpos\n")
        self.clock = Clock()
        self.out = self.root / "out"

    def argv(self, *extra):
        return ["paired_search_diagnostics.py", "--baseline", str(self.engine),
                "--candidate", str(self.engine), "--positions", str(self.positions),
                "--output", str(self.out), "--threads", "1", "--count", "1",
                "--repeats", "1", *extra]

    def manifest(self):
        def reject_constant(value):
            raise ValueError(f"nonstandard JSON constant: {value}")
        return json.loads((self.out / "manifest.json").read_text(), parse_constant=reject_constant)

    @staticmethod
    def fake_measure(lane, threads, positions):
        return [dict(threads=threads, depth=8, nodes=100, wall_nps=1000,
                     elapsed_ms=100, bestmove="7g7f", score=["cp", "0"], pv=["7g7f"])]

    def run_driver(self, extra=(), snapshots=None, sleep=None, host=None, measure=None):
        if snapshots is None:
            snapshots = [IDLE] * 20
        with patch("sys.argv", self.argv(*extra)), patch.object(paired, "host_activity", side_effect=host or snapshots), \
                patch.object(paired.time, "monotonic", side_effect=self.clock.monotonic), \
                patch.object(paired.time, "sleep", side_effect=sleep or self.clock.sleep), \
                patch.object(paired, "measure", side_effect=measure or self.fake_measure) as measured, \
                patch("builtins.print"):
            result = paired.main()
        return result, measured

    def enabled(self, wait="10", poll="1", stable="2"):
        return ("--idle-wait-seconds", wait, "--idle-poll-seconds", poll,
                "--idle-stable-samples", stable)

    def test_default_busy_path_has_no_wait_or_optional_manifest_fields(self):
        result, measured = self.run_driver(snapshots=[BUSY])
        self.assertEqual(result, 2)
        measured.assert_not_called()
        self.assertEqual(self.clock.sleeps, [])
        manifest = self.manifest()
        self.assertNotIn("idle_admission", manifest)
        self.assertEqual(manifest["host_activity_before"], BUSY)
        self.assertFalse((self.out / "rows.jsonl").exists())

    def test_explicit_zero_preserves_configuration_and_manifest_contract(self):
        self.run_driver(snapshots=[BUSY])
        before = self.manifest()
        self.out = self.root / "explicit-zero"
        self.run_driver(extra=("--idle-wait-seconds", "0"), snapshots=[BUSY])
        after = self.manifest()
        self.assertEqual(before["configuration_sha256"], after["configuration_sha256"])
        self.assertNotIn("idle_admission", after)
        self.assertNotIn("completed_searches", after)

    def test_stable_admission_records_initial_busy_and_resets_idle_streak(self):
        snapshots = [BUSY, IDLE, BUSY, IDLE, IDLE, IDLE]
        result, measured = self.run_driver(self.enabled(), snapshots)
        self.assertEqual(result, 0)
        self.assertEqual(measured.call_count, 4)
        admission = self.manifest()["idle_admission"]
        self.assertEqual(admission["status"], "admitted")
        self.assertEqual([s["state"] for s in admission["samples"]], ["busy", "idle", "busy", "idle", "idle"])
        self.assertEqual([s["consecutive_idle_samples"] for s in admission["samples"]], [0, 1, 0, 1, 2])
        self.assertEqual([s["elapsed_seconds"] for s in admission["samples"]], [0, 1, 2, 3, 4])
        self.assertEqual([s["sample_index"] for s in admission["samples"]], list(range(5)))
        self.assertTrue(all(s["observed_at_utc"] for s in admission["samples"]))
        self.assertEqual(self.manifest()["host_activity_before"], IDLE)
        self.assertTrue(self.manifest()["timing_qualified"])
        self.assertEqual(self.manifest()["completed_searches"], 4)

    def test_unknown_and_malformed_snapshots_reset_stability(self):
        invalid = [None, {}, {"available": True, "busy_processes": []},
                   {**IDLE, "load_average": [float("nan"), 0, 0]},
                   {**IDLE, "load_average": [float("inf"), 0, 0]},
                   {**IDLE, "busy_processes": [{}]}, {**IDLE, "available": 1}]
        for index, snapshot in enumerate(invalid):
            with self.subTest(snapshot=snapshot):
                self.out = self.root / f"unknown-{index}"
                result, measured = self.run_driver(self.enabled(), [IDLE, snapshot, IDLE, IDLE, IDLE])
                self.assertEqual(result, 0)
                self.assertEqual(measured.call_count, 4)
                samples = self.manifest()["idle_admission"]["samples"]
                self.assertEqual([s["state"] for s in samples], ["idle", "unknown", "idle", "idle"])
                self.assertEqual(samples[1]["consecutive_idle_samples"], 0)

    def test_samples_are_immutable_copies_of_reused_host_dictionary(self):
        activity = {**BUSY, "busy_processes": [dict(BUSY["busy_processes"][0])]}
        calls = 0
        def host(**kwargs):
            self.assertTrue(kwargs["strict"])
            nonlocal calls
            calls += 1
            if calls > 1:
                activity["busy_processes"].clear()
            return activity
        result, _ = self.run_driver(self.enabled(), host=host)
        self.assertEqual(result, 0)
        samples = self.manifest()["idle_admission"]["samples"]
        self.assertEqual(samples[0]["host_activity"]["busy_processes"], BUSY["busy_processes"])
        self.assertEqual(samples[0]["state"], "busy")
        self.assertEqual(samples[1]["host_activity"]["busy_processes"], [])

    def test_nonfinite_samples_remain_strict_json_in_incremental_manifests(self):
        for index, value in enumerate((float("nan"), float("inf"), -float("inf"))):
            with self.subTest(value=value):
                self.out = self.root / f"strict-{index}"
                snapshots = [{**IDLE, "load_average": [value, 0, 0]}, IDLE, IDLE, IDLE]
                def checked_sleep(seconds):
                    manifest = self.manifest()
                    sample = manifest["idle_admission"]["samples"][0]
                    self.assertEqual(sample["state"], "unknown")
                    self.assertFalse(sample["host_activity"]["available"])
                    self.assertIn("strict JSON", sample["host_activity"]["error"])
                    self.clock.sleep(seconds)
                result, _ = self.run_driver(self.enabled(), snapshots, sleep=checked_sleep)
                self.assertEqual(result, 0)
                self.manifest()

    def test_timeout_leaves_no_rows_and_never_calls_measure(self):
        result, measured = self.run_driver(self.enabled(wait="2", stable="3"), [BUSY] * 4)
        self.assertEqual(result, 2)
        measured.assert_not_called()
        manifest = self.manifest()
        self.assertEqual(manifest["status"], "blocked_host_busy")
        self.assertEqual(manifest["idle_admission"]["status"], "timeout")
        self.assertEqual(manifest["completed_searches"], 0)
        self.assertFalse(manifest["timing_qualified"])
        self.assertFalse((self.out / "rows.jsonl").exists())
        self.assertEqual(self.clock.sleeps, [1, 1])

    def test_snapshot_over_deadline_cannot_admit_even_if_idle_streak_complete(self):
        calls = 0
        def host(**kwargs):
            self.assertTrue(kwargs["strict"])
            nonlocal calls
            calls += 1
            if calls == 2:
                self.clock.now += 2
            return IDLE
        result, measured = self.run_driver(self.enabled(wait="2"), host=host)
        self.assertEqual(result, 2)
        measured.assert_not_called()
        admission = self.manifest()["idle_admission"]
        self.assertEqual(admission["status"], "timeout")
        self.assertEqual(admission["samples"][-1]["elapsed_seconds"], 3)

    def test_overslept_poll_never_samples_or_dispatches_after_deadline(self):
        def late_sleep(seconds):
            self.clock.now += seconds + 5
        result, measured = self.run_driver(self.enabled(wait="2"), [IDLE], sleep=late_sleep)
        self.assertEqual(result, 2)
        measured.assert_not_called()
        self.assertEqual(len(self.manifest()["idle_admission"]["samples"]), 1)

    def test_exact_deadline_last_sample_can_admit(self):
        result, measured = self.run_driver(self.enabled(wait="2", stable="3"))
        self.assertEqual(result, 0)
        self.assertEqual(measured.call_count, 4)
        self.assertEqual(self.manifest()["idle_admission"]["elapsed_seconds"], 2)

    def test_interruption_during_wait_is_durably_incomplete(self):
        def interrupted(seconds):
            manifest = self.manifest()
            self.assertEqual(manifest["status"], "waiting_idle_admission")
            self.assertEqual(manifest["completed_searches"], 0)
            self.assertFalse(manifest["timing_qualified"])
            raise KeyboardInterrupt("user interrupted")
        with self.assertRaises(KeyboardInterrupt):
            self.run_driver(self.enabled(), [IDLE], sleep=interrupted)
        manifest = self.manifest()
        self.assertEqual(manifest["status"], "incomplete")
        self.assertEqual(manifest["idle_admission"]["status"], "incomplete")
        self.assertEqual(manifest["completed_searches"], 0)
        self.assertFalse((self.out / "rows.jsonl").exists())

    def test_initial_snapshot_failure_is_recorded_before_any_measure(self):
        def failed(**kwargs):
            raise RuntimeError("snapshot failed")
        with self.assertRaisesRegex(RuntimeError, "snapshot failed"):
            self.run_driver(self.enabled(), host=failed)
        self.assertEqual(self.manifest()["status"], "incomplete")
        self.assertEqual(self.manifest()["completed_searches"], 0)
        self.assertFalse((self.out / "rows.jsonl").exists())

    def test_frozen_engine_or_corpus_mutation_blocks_dispatch(self):
        for name in ("engine-A", "engine-B", "positions.input"):
            with self.subTest(name=name):
                self.out = self.root / name
                def mutate(seconds):
                    self.clock.sleep(seconds)
                    (self.out / name).write_bytes(b"drift")
                with patch.object(paired, "measure") as measured:
                    with self.assertRaisesRegex(ValueError, "input changed"):
                        self.run_driver(self.enabled(), [IDLE, IDLE], sleep=mutate, measure=measured)
                    measured.assert_not_called()
                self.assertEqual(self.manifest()["status"], "incomplete")
                self.assertFalse((self.out / "rows.jsonl").exists())

    def test_measurement_source_drift_blocks_dispatch(self):
        original = Path.read_bytes
        for filename in ("paired_search_diagnostics.py", "nps_threads.py"):
            with self.subTest(filename=filename):
                self.out = self.root / filename
                changed = False
                def mutate(seconds):
                    nonlocal changed
                    self.clock.sleep(seconds)
                    changed = True
                def read(path):
                    data = original(path)
                    return data + b"drift" if changed and path.name == filename else data
                with patch.object(Path, "read_bytes", read):
                    with self.assertRaisesRegex(ValueError, "input changed"):
                        self.run_driver(self.enabled(), [IDLE, IDLE], sleep=mutate)
                self.assertFalse((self.out / "rows.jsonl").exists())

    def test_original_sources_may_be_removed_after_freeze(self):
        def remove(seconds):
            self.clock.sleep(seconds)
            self.engine.unlink()
            self.positions.unlink()
        result, measured = self.run_driver(self.enabled(), sleep=remove)
        self.assertEqual(result, 0)
        self.assertEqual(measured.call_count, 4)

    def test_configuration_hash_includes_request_but_not_admission_outcomes(self):
        self.run_driver(self.enabled(), [IDLE] * 3)
        first = self.manifest()["configuration_sha256"]
        self.out = self.root / "busy-then-idle"
        self.run_driver(self.enabled(), [BUSY, IDLE, IDLE, IDLE])
        self.assertEqual(first, self.manifest()["configuration_sha256"])
        self.out = self.root / "different-request"
        self.run_driver(self.enabled(wait="20"))
        self.assertNotEqual(first, self.manifest()["configuration_sha256"])

    def test_postflight_busy_still_disqualifies_timing(self):
        result, _ = self.run_driver(self.enabled(), [IDLE, IDLE, BUSY])
        self.assertEqual(result, 0)
        manifest = self.manifest()
        self.assertFalse(manifest["timing_qualified"])
        self.assertEqual(manifest["idle_admission"]["status"], "admitted")

    def test_strict_producer_rejects_incomplete_or_invalid_process_output(self):
        invalid = ["", "PID %CPU COMM\n", "PID %CPU COMM\n42\n",
                   "PID %CPU COMM\n42 nan job\n", "PID %CPU COMM\n42 inf job\n",
                   "PID %CPU COMM\n42 -inf job\n", "PID %CPU COMM\n42 -1 job\n",
                   "PID %CPU COMM\n0 1 job\n", "PID CPU COMM\n42 1 job\n",
                   "PID %CPU COMM\n42 1\n", "PID %CPU COMM\n42 1 job\nbad row\n"]
        for output in invalid:
            with self.subTest(output=output), patch.object(paired.subprocess, "run", return_value=SimpleNamespace(stdout=output)):
                snapshot = paired.host_activity(strict=True)
                self.assertFalse(snapshot["available"])
                self.assertIsNone(snapshot["busy_processes"])
                self.assertEqual(paired.idle_snapshot_state(snapshot), "unknown")

    def test_strict_producer_supports_headers_and_preserves_threshold(self):
        for header in ("PID %CPU COMM", "PID %CPU COMMAND"):
            with self.subTest(header=header), patch.object(paired.subprocess, "run", return_value=SimpleNamespace(
                    stdout=header + "\n23 24.9 quiet\n24 25 busy command\n")), \
                    patch.object(paired.os, "getpid", return_value=1000):
                snapshot = paired.host_activity(strict=True)
                self.assertTrue(snapshot["available"])
                self.assertEqual(snapshot["busy_process_threshold_percent"], 25)
                self.assertEqual(snapshot["busy_processes"], [{"pid": 24, "cpu_percent": 25, "command": "busy command"}])

    def test_default_producer_retains_legacy_row_and_header_behavior(self):
        with patch.object(paired.subprocess, "run", return_value=SimpleNamespace(stdout="PID %CPU COMM\nmalformed\n42 nan job\n")):
            self.assertEqual(paired.host_activity()["busy_processes"], [])
        with patch.object(paired.subprocess, "run", return_value=SimpleNamespace(stdout="PID %CPU COMM\n")):
            self.assertTrue(paired.host_activity()["available"])

    def test_malformed_postflight_disqualifies_and_remains_strict_json(self):
        for index, snapshot in enumerate(({}, None, {**IDLE, "load_average": [float("nan"), 0, 0]},
                                          {**IDLE, "busy_processes": "missing"})):
            with self.subTest(snapshot=snapshot):
                self.out = self.root / f"bad-postflight-{index}"
                result, measured = self.run_driver(self.enabled(), [IDLE, IDLE, snapshot])
                self.assertEqual(result, 0)
                self.assertEqual(measured.call_count, 4)
                manifest = self.manifest()
                self.assertEqual(manifest["status"], "complete")
                self.assertFalse(manifest["timing_qualified"])
                self.assertEqual(manifest["host_activity_before"], IDLE)

    def test_conflicting_or_nonfinite_cli_inputs_fail_before_output_creation(self):
        invalid = [("--depth", "8"), ("--nodes", "100"), ("--diagnostics",), ("--allow-busy",),
                   ("--option", "T_COOP_LATENCY=1"), ("--b-option", "T_COOP_PRESTART_DIAG=1"),
                   ("--option", "T_COOP_DIAG=1"), ("--idle-wait-seconds", "nan"),
                   ("--idle-wait-seconds", "inf"), ("--idle-wait-seconds", "-1"),
                   ("--idle-wait-seconds", "3601"), ("--idle-poll-seconds", "nan"),
                   ("--idle-poll-seconds", "inf"), ("--idle-poll-seconds", "0"),
                   ("--idle-poll-seconds", "61"), ("--idle-stable-samples", "1"),
                   ("--idle-stable-samples", "61"), ("--idle-stable-samples", "2.5"),
                   ("--timeout", "nan"), ("--timeout", "inf"), ("--timeout", "-inf"),
                   ("--idle-wait-seconds", "1", "--idle-stable-samples", "3")]
        for index, conflict in enumerate(invalid):
            with self.subTest(conflict=conflict):
                self.out = self.root / f"invalid-{index}"
                with patch("sys.argv", self.argv(*self.enabled(), *conflict)), patch("sys.stderr"), \
                        patch.object(paired, "measure") as measured:
                    with self.assertRaises(SystemExit) as error:
                        paired.main()
                    self.assertEqual(error.exception.code, 2)
                    measured.assert_not_called()
                self.assertFalse(self.out.exists())


if __name__ == "__main__":
    unittest.main()
