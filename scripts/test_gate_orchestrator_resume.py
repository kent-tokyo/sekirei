#!/usr/bin/env python3
"""Unit test for gate_orchestrator.py's shard_is_alive(), added alongside the
fix for the stale-PID resume gap (a "running" shard left over from a dead
prior process invocation, whose pid may since have been recycled by an
unrelated process on this shared machine, must never be trusted just because
os.kill(pid, 0) succeeds).

Run: python3 scripts/test_gate_orchestrator_resume.py
"""
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(__file__))

from gate_orchestrator import launch_shard, shard_is_alive, verify_weights_loaded


class ShardIsAliveTest(unittest.TestCase):
    def test_dead_pid_is_not_alive(self):
        proc = subprocess.Popen(["true"])
        proc.wait()
        self.assertFalse(shard_is_alive(proc.pid))

    def test_live_pid_of_a_different_command_is_not_a_shard(self):
        # Our own pid is very much alive, but its comm is "python3", not
        # "sekirei-match" -- a recycled pid must not be trusted just because
        # something (anything) is running there.
        self.assertFalse(shard_is_alive(os.getpid()))

    def test_live_pid_named_sekirei_match_is_a_shard(self):
        # Positive path, self-contained (no dependency on a real build): a
        # copy of `sleep` renamed to look like the match binary, so `ps -o
        # comm=` reports a name containing "sekirei-match".
        with tempfile.TemporaryDirectory() as d:
            fake = os.path.join(d, "sekirei-match")
            shutil.copy(shutil.which("sleep"), fake)
            proc = subprocess.Popen([fake, "5"])
            try:
                self.assertTrue(shard_is_alive(proc.pid))
            finally:
                proc.kill()
                proc.wait()


class LaunchShardWeightsTest(unittest.TestCase):
    """Covers the --weights-optional fix (2026-08-24): --weights loads
    identical weights into BOTH engines' argv[1], which is wrong for an
    asymmetric gate (e.g. NNUE candidate vs. material baseline, where only
    one arm should get weights). Omitting --weights must drop --args1/
    --args2 from the launched command entirely, not pass an empty path."""

    def _launch_and_capture_cmd(self, weights):
        cfg = {
            "threads": 1,
            "option1": ["EvalFile=/tmp/candidate.bin"],
            "option2": [],
            "engine_bin": "./target/release/sekirei",
            "weights": weights,
            "byoyomi": 1500,
        }
        shard = {"shard_id": 0, "start_pos": 0, "end_pos": 1}
        with tempfile.TemporaryDirectory() as outdir:
            with mock.patch("gate_orchestrator.subprocess.Popen") as popen:
                popen.return_value.pid = 12345
                launch_shard(cfg, outdir, shard, ["startpos"])
                return popen.call_args[0][0]  # the cmd list

    def test_weights_omitted_drops_args1_args2(self):
        cmd = self._launch_and_capture_cmd(None)
        self.assertNotIn("--args1", cmd)
        self.assertNotIn("--args2", cmd)
        self.assertIn("--engine-option1", cmd)
        self.assertIn("EvalFile=/tmp/candidate.bin", cmd)

    def test_weights_given_sets_args1_args2_on_both_engines(self):
        cmd = self._launch_and_capture_cmd("shared.bin")
        args1_idx = cmd.index("--args1")
        args2_idx = cmd.index("--args2")
        self.assertEqual(cmd[args1_idx + 1], "shared.bin")
        self.assertEqual(cmd[args2_idx + 1], "shared.bin")


class VerifyWeightsLoadedTest(unittest.TestCase):
    def test_idempotent_second_game_reload_is_not_a_failure(self):
        with tempfile.TemporaryDirectory() as outdir:
            with open(os.path.join(outdir, "shard_0000.stderr.log"), "w") as f:
                f.write("NNUE weights loaded\n")
                f.write(
                    "info string weight load failed: "
                    "NNUE weights are already loaded for this process\n"
                )
            shard = {"shard_id": 0}
            self.assertIsNone(verify_weights_loaded(outdir, shard))

    def test_actual_weight_load_failure_is_still_rejected(self):
        with tempfile.TemporaryDirectory() as outdir:
            with open(os.path.join(outdir, "shard_0000.stderr.log"), "w") as f:
                f.write("info string weight load failed: invalid checkpoint\n")
            shard = {"shard_id": 0}
            self.assertFalse(verify_weights_loaded(outdir, shard))


if __name__ == "__main__":
    unittest.main()
