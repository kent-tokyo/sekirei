#!/usr/bin/env python3
"""Shared-host gate scheduling must never select game outcomes."""
import subprocess
import unittest
from pathlib import Path
from unittest.mock import patch

from scripts import nnue_gate_host as H


class HostPolicyTests(unittest.TestCase):
    def sample(self, cpu=0, free=2048):
        return {"cpu_observation_available": True, "foreign_total_cpu": cpu, "free_mib": free}

    def test_os_cleanup_and_one_core_compiler_do_not_block_ten_core_host(self):
        p = H.HostPolicy()
        # Actual aborted run: deleted=94.4%, total foreign CPU=127.4%.
        self.assertTrue(p.admission(self.sample(127.4)))
        self.assertTrue(p.admission(self.sample(100)))
        self.assertEqual(p.max_foreign_cpu_percent, 300)

    def test_sustained_foreign_work_waits_only_before_next_pair(self):
        p = H.HostPolicy()
        self.assertTrue(p.admission(self.sample(300)))
        self.assertFalse(p.admission(self.sample(300.1)))
        self.assertEqual(H.active_pair_action(self.sample(900)), "finish_and_retain_pair")

    def test_admission_needs_three_consecutive_samples(self):
        p = H.HostPolicy()
        quiet = 0
        for cpu, expected in [(0, 1), (127.4, 2), (500, 0), (0, 1), (0, 2), (0, 3), (0, 3)]:
            quiet = p.next_stable_count(quiet, self.sample(cpu))
            self.assertEqual(quiet, expected)

    def test_low_disk_waits_before_pair_without_censoring_active_pair(self):
        p = H.HostPolicy()
        self.assertTrue(p.admission(self.sample(free=512)))
        self.assertFalse(p.admission(self.sample(free=511)))
        self.assertEqual(H.active_pair_action(self.sample(free=0)), "finish_and_retain_pair")

    def test_missing_or_nonfinite_observation_defers_admission(self):
        p = H.HostPolicy()
        for field, value in [("foreign_total_cpu", None), ("foreign_total_cpu", float("nan")),
                             ("foreign_total_cpu", -1), ("foreign_total_cpu", True),
                             ("cpu_observation_available", False), ("free_mib", None)]:
            sample = self.sample()
            sample[field] = value
            with self.subTest(field=field, value=value):
                self.assertFalse(p.admission(sample))
                self.assertEqual(H.active_pair_action(sample), "finish_and_retain_pair")

    def test_process_tree_excludes_owned_runner_and_both_engines(self):
        rows = H.parse_processes("PID PPID %CPU COMM\n1 0 2 launchd\n10 1 1 python\n"
                                 "11 10 2 match\n12 11 100 engine\n13 11 100 engine\n"
                                 "20 1 94.4 /System/deleted\n")
        foreign = H.foreign_processes(rows, 10)
        self.assertEqual([row["pid"] for row in foreign], [1, 20])
        self.assertAlmostEqual(sum(row["cpu"] for row in foreign), 96.4)

    def test_invalid_process_observation_is_not_idle(self):
        for text in ["", "PID PPID CPU COMM\n", "header\n1 0 nan deleted\n",
                     "header\n1 0 -1 deleted\n", "header\n1 0 2\n"]:
            with self.subTest(text=text), self.assertRaises(ValueError):
                H.parse_processes(text)

    def test_ps_failure_is_recorded_but_does_not_censor_active_pair(self):
        with patch.object(H.subprocess, "run", side_effect=subprocess.TimeoutExpired("ps", 10)):
            sample = H.capture(Path.cwd())
        self.assertFalse(sample["admitted"])
        self.assertEqual(sample["cpu_observation_error"], "TimeoutExpired")
        self.assertEqual(H.active_pair_action(sample), "finish_and_retain_pair")

    def test_policy_validation(self):
        for kwargs in [{"physical_cores": 0}, {"physical_cores": True},
                       {"max_foreign_core_fraction": float("nan")},
                       {"max_foreign_core_fraction": 0}, {"minimum_free_mib": 0},
                       {"stable_samples": 0}, {"poll_seconds": 0}]:
            with self.subTest(kwargs=kwargs), self.assertRaises(ValueError):
                H.HostPolicy(**kwargs)


if __name__ == "__main__":
    unittest.main()
