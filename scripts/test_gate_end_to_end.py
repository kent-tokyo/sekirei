#!/usr/bin/env python3
"""End-to-end integration tests for gate_phase_a2_weight_ab.py: drives the
real CLI (subprocess) against the real ./target/release/sekirei-match binary
and scripts/test_fixtures/fake_usi_engine.py stand-in engines. Unlike
test_gate_phase_a2_weight_ab.py's unit tests (pure functions, no subprocess),
these prove the whole pipeline -- permutation, manifest, shard launch, real
TimeForfeit detection through the actual compiled binary, counters,
stop-rule, resume, quarantine -- works together for real, not just in
isolation.

Requires ./target/release/sekirei-match to be built first:
  cargo build --release -p sekirei-match-runner

Run: python3 scripts/test_gate_end_to_end.py
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import tomllib
import unittest

_REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
_GATE_SCRIPT = os.path.join(_REPO_ROOT, "scripts", "gate_phase_a2_weight_ab.py")
_FAKE_ENGINE = os.path.join(_REPO_ROOT, "scripts", "test_fixtures", "fake_usi_engine.py")
_MATCH_BIN = os.path.join(_REPO_ROOT, "target", "release", "sekirei-match")


def _run_gate(outdir, weights1, weights2, corpus, max_positions, shard_positions=1, parallel=1, timeout=60):
    cmd = [
        sys.executable, _GATE_SCRIPT, "run",
        "--outdir", outdir,
        "--threads", "1", "--parallel", str(parallel), "--byoyomi", "50",
        "--shard-positions", str(shard_positions), "--max-positions", str(max_positions),
        "--max-swap-pct", "92",  # preflight §12's resolved recommendation for this shared machine
        "--engine-bin", _FAKE_ENGINE,
        "--weights1", weights1, "--weights2", weights2,
        "--corpus", corpus,
        "--elo0", "0", "--elo1", "20", "--alpha", "0.05", "--beta", "0.05",
    ]
    return subprocess.run(cmd, cwd=_REPO_ROOT, capture_output=True, text=True, timeout=timeout)


@unittest.skipUnless(
    os.path.exists(_MATCH_BIN),
    f"{_MATCH_BIN} not built -- run: cargo build --release -p sekirei-match-runner",
)
class EndToEndTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="gate_e2e_test_")

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def _mode_file(self, name, mode):
        path = os.path.join(self.tmp, name)
        with open(path, "w") as f:
            f.write(mode)
        return path

    def _corpus(self, n_positions):
        path = os.path.join(self.tmp, "corpus.sfen")
        with open(path, "w") as f:
            for _ in range(n_positions):
                f.write("startpos\n")
        return path

    def test_genuine_time_forfeit_contaminates_and_quarantines(self):
        # normal_then_resign vs hang: proves the real go()-deadline path
        # (engine.rs's map_recv_result distinguishing Timeout from
        # Disconnected) fires through actual subprocess execution, not just
        # the pure-function unit tests -- and that CONTAMINATED halts and
        # quarantines the run directory as §3 requires.
        weights1 = self._mode_file("w1.txt", "normal_then_resign")
        weights2 = self._mode_file("w2.txt", "hang")
        corpus = self._corpus(1)
        outdir = os.path.join(self.tmp, "gate_out")

        result = _run_gate(outdir, weights1, weights2, corpus, max_positions=1, timeout=90)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("time_forfeits': 2", result.stdout)
        self.assertIn("CONTAMINATED", result.stdout)
        self.assertIn("quarantined", result.stdout)

        self.assertFalse(os.path.exists(outdir), "contaminated outdir must be renamed away")
        quarantined = outdir + "_contaminated"
        self.assertTrue(os.path.exists(quarantined))

        # The underlying "(time forfeit)" tag itself lives in the shard's own
        # stdout log (the real sekirei-match subprocess's per-game summary
        # line), not the orchestrator's own console output -- check it
        # directly so this test proves the tag was genuinely emitted, not
        # just that the gate's counter arithmetic produced a 2.
        with open(os.path.join(quarantined, "shard_0000.stdout.log")) as f:
            shard_stdout = f.read()
        self.assertEqual(shard_stdout.count("(time forfeit)"), 2)

        with open(os.path.join(quarantined, "manifest.toml"), "rb") as f:
            manifest = tomllib.load(f)
        self.assertEqual(manifest["progress"][-1]["verdict"], "CONTAMINATED")
        self.assertEqual(manifest["progress"][-1]["time_forfeits"], 2)
        self.assertEqual(manifest["progress"][-1]["illegal_moves"], 0)

        with open(os.path.join(quarantined, "combined.jsonl")) as f:
            records = [json.loads(line) for line in f if line.strip()]
        self.assertEqual(len(records), 2)

    def test_clean_run_reaches_inconclusive_and_resume_is_idempotent(self):
        weights1 = self._mode_file("w1.txt", "resign_immediately")
        weights2 = self._mode_file("w2.txt", "resign_immediately")
        corpus = self._corpus(2)
        outdir = os.path.join(self.tmp, "gate_out")

        first = _run_gate(outdir, weights1, weights2, corpus, max_positions=2, parallel=2, timeout=60)
        self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
        self.assertIn("INCONCLUSIVE", first.stdout)

        with open(os.path.join(outdir, "manifest.toml"), "rb") as f:
            manifest_after_first = tomllib.load(f)
        entries_after_first = len(manifest_after_first["progress"])

        # Resume twice more on the same, already-finished outdir -- must not
        # re-process games, re-decide the verdict, or append duplicate
        # manifest snapshots each time (regression: an earlier version of
        # this script appended a fresh "inconclusive" snapshot on every
        # no-op resume since the append wasn't gated on decisive_verdict
        # still being None).
        for _ in range(2):
            resumed = _run_gate(outdir, weights1, weights2, corpus, max_positions=2, parallel=2, timeout=30)
            self.assertEqual(resumed.returncode, 0, resumed.stdout + resumed.stderr)

        with open(os.path.join(outdir, "manifest.toml"), "rb") as f:
            manifest_after_resumes = tomllib.load(f)
        self.assertEqual(len(manifest_after_resumes["progress"]), entries_after_first)

    def test_resume_after_weight_file_changes_refuses_to_continue(self):
        weights1 = self._mode_file("w1.txt", "resign_immediately")
        weights2 = self._mode_file("w2.txt", "resign_immediately")
        corpus = self._corpus(1)
        outdir = os.path.join(self.tmp, "gate_out")

        first = _run_gate(outdir, weights1, weights2, corpus, max_positions=1, timeout=40)
        self.assertEqual(first.returncode, 0, first.stdout + first.stderr)

        with open(weights1, "w") as f:
            f.write("resign_immediately_but_different_content")

        resumed = _run_gate(outdir, weights1, weights2, corpus, max_positions=1, timeout=20)
        self.assertNotEqual(resumed.returncode, 0)
        self.assertIn("resume mismatch", resumed.stdout + resumed.stderr)
        self.assertIn("candidate_weight_sha256", resumed.stdout + resumed.stderr)


if __name__ == "__main__":
    unittest.main(verbosity=2)
