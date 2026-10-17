"""Capacity admission uses fake engines, disk observations and search calls only."""
import json
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory
from types import SimpleNamespace
from unittest.mock import patch

import paired_search_diagnostics as paired
import sekirei_tools
from sekirei_tools import artifacts
from test_idle_search_admission import BUSY, IDLE, Clock


MIB = 1024 * 1024


class IdleCapacityAdmissionTests(unittest.TestCase):
    def setUp(self):
        self.tmp = TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.engine = self.root / "engine"
        self.engine.write_bytes(b"unexecutable fixture, never launched")
        self.positions = self.root / "positions"
        self.positions.write_text("startpos\n")
        self.out = self.root / "out"
        self.clock = Clock()

    def argv(self, *extra):
        return ["paired_search_diagnostics.py", "--baseline", str(self.engine),
                "--candidate", str(self.engine), "--positions", str(self.positions),
                "--output", str(self.out), "--threads", "1", "--count", "1",
                "--repeats", "1", *extra]

    @staticmethod
    def enabled(*extra):
        return ("--idle-wait-seconds", "10", "--idle-poll-seconds", "1",
                "--idle-stable-samples", "2", "--idle-min-free-mib", "1", *extra)

    def manifest(self):
        def reject_constant(value):
            raise ValueError(f"nonstandard JSON constant: {value}")
        return json.loads((self.out / "manifest.json").read_text(), parse_constant=reject_constant)

    @staticmethod
    def fake_measure(lane, threads, positions):
        return [dict(threads=threads, depth=8, nodes=100, wall_nps=1000,
                     elapsed_ms=100, bestmove="7g7f", score=["cp", "0"], pv=["7g7f"])]

    def run_driver(self, extra=None, free=None, hosts=None, sleep=None, measure=None):
        if extra is None:
            extra = self.enabled()
        if free is None:
            free = [MIB] * 30
        def usage(_path):
            value = next(values)
            if isinstance(value, BaseException):
                raise value
            return SimpleNamespace(total=10 * MIB, used=0, free=value)
        values = iter(free)
        with patch("sys.argv", self.argv(*extra)), \
                patch.object(paired, "host_activity", side_effect=hosts or [IDLE] * 30), \
                patch.object(paired.shutil, "disk_usage", side_effect=usage) as disks, \
                patch.object(paired.time, "monotonic", side_effect=self.clock.monotonic), \
                patch.object(paired.time, "sleep", side_effect=sleep or self.clock.sleep), \
                patch.object(paired, "measure", side_effect=measure or self.fake_measure) as measured, \
                patch("builtins.print"):
            result = paired.main()
        return result, measured, disks

    def test_low_capacity_resets_joint_streak_and_recovers_on_fifth_sample(self):
        result, measured, _ = self.run_driver(free=[MIB - 1, MIB, MIB - 1, MIB, MIB] + [MIB] * 10)
        self.assertEqual(result, 0)
        self.assertEqual(measured.call_count, 4)
        manifest = self.manifest()
        admission = manifest["idle_admission"]
        self.assertEqual(admission["request"]["minimum_free_mib"], 1)
        self.assertEqual([s["state"] for s in admission["samples"]], ["idle"] * 5)
        self.assertEqual([s["consecutive_idle_samples"] for s in admission["samples"]], [1, 2, 3, 4, 5])
        self.assertEqual([s["qualified"] for s in admission["samples"]], [False, True, False, True, True])
        self.assertEqual([s["consecutive_qualified_samples"] for s in admission["samples"]], [0, 1, 0, 1, 2])
        self.assertEqual(admission["consecutive_qualified_samples"], 2)
        self.assertEqual([s["capacity"]["free_bytes"] for s in admission["samples"]], [MIB - 1, MIB, MIB - 1, MIB, MIB])
        self.assertTrue(manifest["capacity_before_dispatch"]["floor_pass"])
        self.assertEqual([s["sequence"] for s in manifest["capacity_before_search"]], [0, 1, 2, 3])
        self.assertTrue(all(s["capacity"]["floor_pass"] for s in manifest["capacity_before_search"]))
        self.assertTrue(manifest["capacity_after"]["floor_pass"])
        self.assertTrue(manifest["timing_qualified"])

    def test_busy_cpu_resets_joint_streak_even_when_capacity_passes(self):
        result, measured, _ = self.run_driver(hosts=[IDLE, BUSY, IDLE, IDLE, IDLE])
        self.assertEqual(result, 0)
        self.assertEqual(measured.call_count, 4)
        samples = self.manifest()["idle_admission"]["samples"]
        self.assertEqual([s["state"] for s in samples], ["idle", "busy", "idle", "idle"])
        self.assertEqual([s["consecutive_qualified_samples"] for s in samples], [1, 0, 1, 2])

    def test_unknown_cpu_resets_joint_streak_even_when_capacity_passes(self):
        result, _, _ = self.run_driver(hosts=[IDLE, {}, IDLE, IDLE, IDLE])
        self.assertEqual(result, 0)
        samples = self.manifest()["idle_admission"]["samples"]
        self.assertEqual(samples[1]["state"], "unknown")
        self.assertFalse(samples[1]["qualified"])
        self.assertEqual([s["consecutive_qualified_samples"] for s in samples], [1, 0, 1, 2])

    def test_unknown_capacity_resets_joint_streak_without_changing_cpu_streak(self):
        result, _, _ = self.run_driver(free=[MIB, OSError("disk unavailable"), MIB, MIB] + [MIB] * 10)
        self.assertEqual(result, 0)
        samples = self.manifest()["idle_admission"]["samples"]
        self.assertFalse(samples[1]["capacity"]["available"])
        self.assertFalse(samples[1]["capacity"]["floor_pass"])
        self.assertEqual([s["consecutive_idle_samples"] for s in samples], [1, 2, 3, 4])
        self.assertEqual([s["consecutive_qualified_samples"] for s in samples], [1, 0, 1, 2])

    def test_capacity_snapshot_boundary_and_output_filesystem(self):
        for free, expected in ((2 * MIB - 1, False), (2 * MIB, True), (2 * MIB + 1, True)):
            with self.subTest(free=free), patch.object(paired.shutil, "disk_usage", return_value=SimpleNamespace(free=free)) as usage:
                snapshot = paired.capacity_snapshot(self.root, 2)
                usage.assert_called_once_with(self.root)
                self.assertIs(snapshot["available"], True)
                self.assertEqual(snapshot["path"], str(self.root.resolve()))
                self.assertEqual(snapshot["minimum_free_bytes"], 2 * MIB)
                self.assertEqual(snapshot["free_bytes"], free)
                self.assertIs(snapshot["floor_pass"], expected)

    def test_capacity_snapshot_rejects_malformed_free_values(self):
        for free in (True, False, 1.0, float("nan"), float("inf"), -1, "1048576", None):
            with self.subTest(free=free), patch.object(paired.shutil, "disk_usage", return_value=SimpleNamespace(free=free)):
                snapshot = paired.capacity_snapshot(self.root, 1)
                self.assertIs(snapshot["available"], False)
                self.assertIs(snapshot["floor_pass"], False)
                self.assertIn("error", snapshot)
                json.dumps(snapshot, allow_nan=False)

    def test_capacity_snapshot_oserror_is_unknown_and_fail_closed(self):
        with patch.object(paired.shutil, "disk_usage", side_effect=OSError("unavailable")):
            snapshot = paired.capacity_snapshot(self.root, 1)
        self.assertFalse(snapshot["available"])
        self.assertFalse(snapshot["floor_pass"])
        self.assertIn("unavailable", snapshot["error"])

    def test_capacity_timeout_never_dispatches_or_creates_rows(self):
        result, measured, _ = self.run_driver(self.enabled("--idle-wait-seconds", "2"), free=[MIB - 1] * 3)
        self.assertEqual(result, 2)
        measured.assert_not_called()
        manifest = self.manifest()
        self.assertEqual(manifest["idle_admission"]["status"], "timeout")
        self.assertEqual(manifest["completed_searches"], 0)
        self.assertFalse(manifest["timing_qualified"])
        self.assertFalse((self.out / "rows.jsonl").exists())

    def test_invalid_capacity_cli_fails_before_output_creation(self):
        invalid = [("--idle-min-free-mib", "1"), ("--idle-min-free-mib", "-1"),
                   (*self.enabled(), "--idle-min-free-mib", "-1"),
                   (*self.enabled(), "--idle-min-free-mib", "1.5"),
                   (*self.enabled(), "--idle-min-free-mib", "nan"),
                   (*self.enabled(), "--idle-min-free-mib", "inf"),
                   (*self.enabled(), "--idle-wait-seconds", "0")]
        for index, extra in enumerate(invalid):
            with self.subTest(extra=extra):
                self.out = self.root / f"invalid-{index}"
                with patch("sys.argv", self.argv(*extra)), patch("sys.stderr"), \
                        patch.object(paired, "measure") as measured, \
                        patch.object(paired.shutil, "disk_usage") as disk:
                    with self.assertRaises(SystemExit) as error:
                        paired.main()
                self.assertEqual(error.exception.code, 2)
                measured.assert_not_called()
                disk.assert_not_called()
                self.assertFalse(self.out.exists())

    def test_capacity_loss_after_input_rehash_blocks_first_dispatch(self):
        original = paired.verify_admission_inputs
        rehashed = False
        def verify(*args):
            nonlocal rehashed
            original(*args)
            rehashed = True
        def usage(_path):
            return SimpleNamespace(free=MIB - 1 if rehashed else MIB)
        with patch.object(paired, "verify_admission_inputs", side_effect=verify), \
                patch.object(paired.shutil, "disk_usage", side_effect=usage), \
                patch("sys.argv", self.argv(*self.enabled())), \
                patch.object(paired, "host_activity", return_value=IDLE), \
                patch.object(paired.time, "monotonic", side_effect=self.clock.monotonic), \
                patch.object(paired.time, "sleep", side_effect=self.clock.sleep), \
                patch.object(paired, "measure") as measured, patch("builtins.print"):
            result = paired.main()
        self.assertEqual(result, 2)
        measured.assert_not_called()
        manifest = self.manifest()
        self.assertEqual(manifest["status"], "blocked_capacity_before_dispatch")
        self.assertEqual(manifest["completed_searches"], 0)
        self.assertFalse(manifest["capacity_before_dispatch"]["floor_pass"])
        self.assertFalse((self.out / "rows.jsonl").exists())

    def test_mid_abba_capacity_loss_stops_all_later_measurements(self):
        for index, bad in enumerate((MIB - 1, OSError("disk unknown"))):
            with self.subTest(bad=bad):
                self.out = self.root / f"mid-{index}"
                with self.assertRaisesRegex(ValueError, "capacity"):
                    self.run_driver(free=[MIB] * 5 + [bad])
                manifest = self.manifest()
                self.assertEqual(manifest["status"], "incomplete")
                self.assertEqual(manifest["completed_searches"], 2)
                self.assertEqual(len((self.out / "rows.jsonl").read_text().splitlines()), 2)
                checks = manifest["capacity_before_search"]
                self.assertEqual([s["sequence"] for s in checks], [0, 1, 2])
                self.assertFalse(checks[-1]["capacity"]["floor_pass"])
                self.assertFalse(manifest["timing_qualified"])
                self.assertFalse((self.out / "summary.json").exists())

    def test_postflight_low_or_unknown_capacity_keeps_complete_but_disqualifies(self):
        for index, bad in enumerate((MIB - 1, OSError("disk unknown"))):
            with self.subTest(bad=bad):
                self.out = self.root / f"postflight-{index}"
                result, measured, _ = self.run_driver(free=[MIB] * 7 + [bad])
                self.assertEqual(result, 0)
                self.assertEqual(measured.call_count, 4)
                manifest = self.manifest()
                self.assertEqual(manifest["status"], "complete")
                self.assertEqual(manifest["completed_searches"], 4)
                self.assertFalse(manifest["timing_qualified"])
                self.assertFalse(manifest["capacity_after"]["floor_pass"])

    def test_default_and_explicit_zero_preserve_legacy_source_hashes_and_no_disk_calls(self):
        manifests = []
        for index, extra in enumerate(((), ("--idle-min-free-mib", "0"))):
            self.out = self.root / f"zero-{index}"
            result, measured, disk = self.run_driver(extra=extra)
            self.assertEqual(result, 0)
            self.assertEqual(measured.call_count, 4)
            disk.assert_not_called()
            manifest = self.manifest()
            self.assertNotIn("idle_admission", manifest)
            self.assertFalse(any(key.startswith("capacity_") for key in manifest))
            self.assertEqual(set(manifest["measurement_source_sha256"]), {"paired_search_diagnostics.py", "nps_threads.py"})
            manifests.append(manifest)
        self.assertEqual(manifests[0]["configuration_sha256"], manifests[1]["configuration_sha256"])

    def test_idle_only_and_explicit_zero_capacity_share_legacy_admission_contract(self):
        manifests = []
        idle = self.enabled()[:-2]
        for index, extra in enumerate((idle, (*idle, "--idle-min-free-mib", "0"))):
            self.out = self.root / f"idle-zero-{index}"
            result, _, disk = self.run_driver(extra=extra)
            self.assertEqual(result, 0)
            disk.assert_not_called()
            manifest = self.manifest()
            self.assertNotIn("minimum_free_mib", manifest["idle_admission"]["request"])
            self.assertNotIn("consecutive_qualified_samples", manifest["idle_admission"])
            for sample in manifest["idle_admission"]["samples"]:
                self.assertNotIn("capacity", sample)
                self.assertNotIn("qualified", sample)
                self.assertNotIn("consecutive_qualified_samples", sample)
            self.assertFalse(any(key.startswith("capacity_") for key in manifest))
            self.assertTrue(any(name.endswith("sekirei_tools/artifacts.py") for name in manifest["measurement_source_sha256"]))
            self.assertTrue(any(name.endswith("sekirei_tools/__init__.py") for name in manifest["measurement_source_sha256"]))
            manifests.append(manifest)
        self.assertEqual(manifests[0]["configuration_sha256"], manifests[1]["configuration_sha256"])

    def test_imported_helper_and_package_paths_must_match_fingerprinted_sources(self):
        shadow = self.root / "shadow.py"
        shadow.write_text("# synthetic unrelated module path\n")
        for index, module in enumerate((artifacts, sekirei_tools)):
            with self.subTest(module=module.__name__):
                self.out = self.root / f"shadow-{index}"
                with patch.object(module, "__file__", str(shadow)), \
                        patch.object(paired, "measure") as measured:
                    with self.assertRaises(ValueError):
                        self.run_driver(measure=measured)
                    measured.assert_not_called()
                self.assertFalse((self.out / "manifest.json").exists())
                self.assertFalse((self.out / "rows.jsonl").exists())

    def test_idle_helper_and_package_source_drift_blocks_dispatch(self):
        original = Path.read_bytes
        for index, filename in enumerate(("artifacts.py", "__init__.py")):
            with self.subTest(filename=filename):
                self.out = self.root / f"helper-drift-{index}"
                changed = False
                def mutate(seconds):
                    nonlocal changed
                    self.clock.sleep(seconds)
                    changed = True
                def read(path):
                    data = original(path)
                    if changed and path.parent.name == "sekirei_tools" and path.name == filename:
                        return data + b"drift"
                    return data
                with patch.object(Path, "read_bytes", read), patch.object(paired, "measure") as measured:
                    with self.assertRaisesRegex(ValueError, "input changed"):
                        self.run_driver(sleep=mutate, measure=measured)
                    measured.assert_not_called()
                manifest = self.manifest()
                self.assertEqual(manifest["status"], "incomplete")
                self.assertEqual(manifest["completed_searches"], 0)
                self.assertFalse((self.out / "rows.jsonl").exists())

    def test_search_time_frozen_input_drift_fails_postflight_after_actual_four_searches(self):
        for index, name in enumerate(("engine-A", "positions.input")):
            with self.subTest(name=name):
                self.out = self.root / f"search-drift-{index}"
                calls = 0
                def mutate(lane, threads, positions):
                    nonlocal calls
                    calls += 1
                    if calls == 1:
                        (self.out / name).write_bytes(b"changed after first synthetic search")
                    return self.fake_measure(lane, threads, positions)
                with patch.object(paired, "measure", side_effect=mutate) as measured:
                    with self.assertRaisesRegex(ValueError, "input changed"):
                        self.run_driver(measure=measured)
                    self.assertEqual(measured.call_count, 4)
                manifest = self.manifest()
                self.assertEqual(manifest["status"], "incomplete")
                self.assertEqual(manifest["completed_searches"], 4)
                self.assertEqual(len((self.out / "rows.jsonl").read_text().splitlines()), 4)
                self.assertFalse(manifest["timing_qualified"])

    def test_capacity_floor_changes_configuration_hash_but_observations_do_not(self):
        hashes = []
        for index, (extra, free) in enumerate(((self.enabled(), [MIB] * 20),
                (self.enabled(), [0, MIB, MIB] + [MIB] * 20),
                (self.enabled("--idle-min-free-mib", "2"), [2 * MIB] * 20))):
            self.out = self.root / f"config-{index}"
            result, _, _ = self.run_driver(extra=extra, free=free)
            self.assertEqual(result, 0)
            hashes.append(self.manifest()["configuration_sha256"])
        self.assertEqual(hashes[0], hashes[1])
        self.assertNotEqual(hashes[0], hashes[2])

    def test_persistent_manifest_fsync_failure_preserves_last_valid_json_and_cleans_temp(self):
        saved = None
        failure = OSError("persistent fsync failure")
        def fail_after_first_sample(seconds):
            nonlocal saved
            saved = (self.out / "manifest.json").read_bytes()
            self.assertEqual(self.manifest()["status"], "waiting_idle_admission")
            self.clock.sleep(seconds)
            def failed_fsync(_fd):
                raise failure
            artifacts.os.fsync = failed_fsync
        original = artifacts.os.fsync
        self.addCleanup(setattr, artifacts.os, "fsync", original)
        with patch.object(paired, "measure") as measured:
            with self.assertRaisesRegex(OSError, "persistent fsync failure"):
                self.run_driver(sleep=fail_after_first_sample, measure=measured)
            measured.assert_not_called()
        self.assertIsNotNone(saved)
        self.assertEqual((self.out / "manifest.json").read_bytes(), saved)
        self.assertEqual(self.manifest()["completed_searches"], 0)
        self.assertEqual(list(self.out.glob(".manifest.json.*")), [])
        self.assertFalse((self.out / "rows.jsonl").exists())


if __name__ == "__main__":
    unittest.main()
