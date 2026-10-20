#!/usr/bin/env python3
"""Process-level regression tests for the one-shot A/B gate supervisor."""

import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from types import SimpleNamespace

ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT))

from ab_match_supervisor import _inputs


class EvaluatorPinTest(unittest.TestCase):
    def test_supervisor_pins_both_declared_halfkp_files(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            candidate, baseline = base / "candidate", base / "baseline"
            candidate.write_bytes(b"candidate"); baseline.write_bytes(b"baseline")
            args = SimpleNamespace(mode="selfplay", engine_a=None, engine_b=None,
                                   match_binary=None, openings=None, evalfile="none",
                                   candidate_evalfile=str(candidate), baseline_evalfile=str(baseline),
                                   openings_manifest=None, gate_observation_declaration=None,
                                   option_a=[], option_b=[])
            before = _inputs(args)
            self.assertEqual(before["candidate_evaluator"]["path"], str(candidate.resolve()))
            self.assertEqual(before["baseline_evaluator"]["path"], str(baseline.resolve()))
            self.assertNotEqual(before["candidate_evaluator"]["sha256"], before["baseline_evaluator"]["sha256"])
            baseline.write_bytes(b"changed baseline")
            after = _inputs(args)
            self.assertEqual(before["candidate_evaluator"], after["candidate_evaluator"])
            self.assertNotEqual(before["baseline_evaluator"], after["baseline_evaluator"])

FAKE_MATCH = r'''#!/usr/bin/env python3
import json
from pathlib import Path
import sys
import time

args = sys.argv[1:]
raw = Path(args[args.index("--json") + 1])
kifu = Path(args[args.index("--output") + 1])
kifu.mkdir()
(kifu / "game0001.txt").write_text("fixture game\n", encoding="utf-8")
raw.with_suffix(".jsonl").write_text(json.dumps({"id": "game0001", "result": "candidate_win"}) + "\n", encoding="utf-8")
raw.write_text(json.dumps({"status": "complete" if "--fixture-complete" in args else "running", "games": 1, "engine1_wins": 1, "engine2_wins": 0, "draws": 0, "artifact_files_expected": 1, "artifact_write_failures": [], "invalid_games": []}) + "\n", encoding="utf-8")
print("→ Engine1 Win", flush=True)
if "--fixture-complete" not in args:
    time.sleep(30)
'''


@unittest.skipUnless(os.name == "posix", "POSIX process-group supervision")
class SupervisorTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.out = self.base / "out"
        self.result = self.base / "result.json"
        self.channel = self.base / "result.json.supervisor"
        self.match = self.base / "fake-match"
        self.match.write_text(FAKE_MATCH, encoding="utf-8")
        self.match.chmod(0o755)
        self.engine = self.base / "engine"
        self.engine.write_text("engine fixture\n", encoding="utf-8")
        self.openings = self.base / "openings.sfen"
        self.openings.write_text("startpos\n", encoding="utf-8")
        self.processes = []
        self.addCleanup(self._stop_processes)

    def _stop_processes(self):
        for proc in self.processes:
            if proc.poll() is None:
                proc.kill()
            proc.wait(timeout=5)
        registration = self.channel / "runner.json"
        if registration.exists():
            try:
                os.killpg(json.loads(registration.read_text())["pgid"], signal.SIGKILL)
            except ProcessLookupError:
                pass

    def _launch(self, complete=False):
        argv = [sys.executable, str(ROOT / "run_ab_match.py"), "selfplay",
                "--engine-a", str(self.engine), "--engine-b", str(self.engine),
                "--match-binary", str(self.match), "--openings", str(self.openings),
                "--out-dir", str(self.out), "--result-json", str(self.result),
                "--name", "fixture", "--games", "1", "--supervised"]
        if complete:
            argv.extend(["--option-a", "FixtureComplete=true"])
            # The fake runner sees the option's value via the real match argv.
            self.match.write_text(FAKE_MATCH.replace('--fixture-complete',
                                                    'FixtureComplete=true'),
                                  encoding="utf-8")
        proc = subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        self.processes.append(proc)
        return proc

    def _wait_for_raw(self):
        deadline = time.monotonic() + 12
        while time.monotonic() < deadline:
            if (self.out / "fixture.json").exists() and (self.channel / "runner.json").exists():
                return json.loads((self.channel / "runner.json").read_text())
            time.sleep(0.02)
        self.fail("match fixture never wrote its first durable game")

    def test_wrapper_sigkill_writes_failure_receipt_and_stops_runner(self):
        proc = self._launch()
        registration = self._wait_for_raw()
        raw_bytes = (self.out / "fixture.json").read_bytes()
        os.kill(registration["wrapper_pid"], signal.SIGKILL)
        stdout, stderr = proc.communicate(timeout=15)
        self.assertNotEqual(proc.returncode, 0, (stdout, stderr))
        receipt = json.loads(self.result.read_text())
        self.assertEqual(receipt["status"], "error")
        self.assertEqual(receipt["terminal_state"], "partial")
        self.assertFalse(receipt["evidence"]["complete"])
        self.assertEqual(receipt["evidence"]["wrapper_exit"]["signal_name"], "SIGKILL")
        self.assertEqual(receipt["evidence"]["last_durable_game_count"], 1)
        self.assertIsNone(receipt["result"]["sprt"])
        self.assertIsNone(receipt["elo"])
        self.assertEqual((self.out / "fixture.json").read_bytes(), raw_bytes)
        self.assertIn(receipt["evidence"]["runner_cleanup"],
                      {"stopped", "already_gone", "signal_sent_group_still_visible"})
        second = subprocess.run(proc.args, capture_output=True, text=True, timeout=5)
        self.assertNotEqual(second.returncode, 0)
        self.assertEqual((self.out / "fixture.json").read_bytes(), raw_bytes)

    def test_match_child_sigkill_writes_failure_receipt(self):
        proc = self._launch()
        registration = self._wait_for_raw()
        os.kill(registration["pid"], signal.SIGKILL)
        stdout, stderr = proc.communicate(timeout=15)
        self.assertNotEqual(proc.returncode, 0, (stdout, stderr))
        receipt = json.loads(self.result.read_text())
        self.assertEqual(receipt["status"], "error")
        self.assertFalse(receipt["evidence"]["complete"])
        self.assertEqual(receipt["evidence"]["last_durable_game_count"], 1)
        self.assertIsNone(receipt["result"]["sprt"])

    def test_supervisor_sigterm_stops_gate_and_writes_failure_receipt(self):
        proc = self._launch()
        self._wait_for_raw()
        os.kill(proc.pid, signal.SIGTERM)
        stdout, stderr = proc.communicate(timeout=15)
        self.assertNotEqual(proc.returncode, 0, (stdout, stderr))
        receipt = json.loads(self.result.read_text())
        self.assertEqual(receipt["status"], "error")
        self.assertEqual(receipt["evidence"]["supervisor_stop_signal"], signal.SIGTERM)
        self.assertFalse(receipt["evidence"]["complete"])

    def test_inconsistent_partial_artifacts_are_never_counted(self):
        proc = self._launch()
        registration = self._wait_for_raw()
        (self.out / "fixture.jsonl").write_text("", encoding="utf-8")
        os.kill(registration["wrapper_pid"], signal.SIGKILL)
        stdout, stderr = proc.communicate(timeout=15)
        self.assertNotEqual(proc.returncode, 0, (stdout, stderr))
        receipt = json.loads(self.result.read_text())
        raw = receipt["evidence"]["raw_outputs"]
        self.assertEqual(raw["snapshot_games"], 1)
        self.assertEqual(raw["jsonl_records"], 0)
        self.assertIsNone(raw["verified_games"])
        self.assertEqual(receipt["status"], "error")
        self.assertEqual(receipt["terminal_state"], "failed")
        self.assertIsNone(receipt["result"]["sprt"])

    def test_complete_gate_keeps_normal_result(self):
        proc = self._launch(complete=True)
        stdout, stderr = proc.communicate(timeout=15)
        self.assertEqual(proc.returncode, 0, (stdout, stderr))
        receipt = json.loads(self.result.read_text())
        self.assertEqual(receipt["status"], "completed")
        self.assertTrue(receipt["evidence"]["complete"])
        self.assertEqual(receipt["result"]["games_played"], 1)


if __name__ == "__main__":
    unittest.main()
