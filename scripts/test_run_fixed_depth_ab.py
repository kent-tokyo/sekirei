#!/usr/bin/env python3
"""Unit tests for run_fixed_depth_ab.py's USI capability guard.

Motivation: run 31362228815 silently compared base (SpecTopN USI option
present) against a candidate binary built from a commit that predates the
option -- `setoption name SpecTopN value 0` was accepted-and-ignored by the
candidate rather than rejected, so the two sides actually ran at different
SpecTopN values, producing a bogus 238x node-count outlier unrelated to the
change under test. probe_usi_capabilities()/require_usi_capabilities() exist
to catch that class of silent mismatch before a corpus run starts.

Each test drives a tiny fake "engine" (a shell script standing in for the
real binary) through the real subprocess-based probe -- no cargo build, no
real engine, no network. Matching this repo's scripts/test_gate_resource_preflight.py
convention: stdlib unittest only.

Run: python3 scripts/test_run_fixed_depth_ab.py
"""
import stat
import tempfile
import unittest
from pathlib import Path

from run_fixed_depth_ab import probe_usi_capabilities, require_usi_capabilities

FULL_SUPPORT_SCRIPT = """#!/bin/sh
echo "id name fake-engine"
echo "option name Threads type spin default 1 min 1 max 512"
echo "option name SpecTopN type spin default 3 min 0 max 512"
echo "usiok"
echo "readyok"
exit 0
"""

NO_SPEC_TOP_N_SCRIPT = """#!/bin/sh
echo "id name fake-engine-old"
echo "option name Threads type spin default 1 min 1 max 512"
echo "usiok"
echo "readyok"
exit 0
"""

NO_READYOK_SCRIPT = """#!/bin/sh
echo "option name Threads type spin default 1 min 1 max 512"
echo "option name SpecTopN type spin default 3 min 0 max 512"
echo "usiok"
exit 0
"""


def _write_fake_binary(tmpdir, contents):
    path = Path(tmpdir) / "fake-engine.sh"
    path.write_text(contents)
    path.chmod(path.stat().st_mode | stat.S_IEXEC)
    return path


class ProbeUsiCapabilitiesTests(unittest.TestCase):
    def test_full_support_binary_advertises_both_options_and_handshakes(self):
        with tempfile.TemporaryDirectory() as tmp:
            binary = _write_fake_binary(tmp, FULL_SUPPORT_SCRIPT)
            caps = probe_usi_capabilities(binary, threads=1, spec_top_n=0, timeout_s=5)
            self.assertIn("Threads", caps["advertised_options"])
            self.assertIn("SpecTopN", caps["advertised_options"])
            self.assertTrue(caps["saw_usiok"])
            self.assertTrue(caps["saw_readyok"])

    def test_pre_option_binary_does_not_advertise_spec_top_n(self):
        with tempfile.TemporaryDirectory() as tmp:
            binary = _write_fake_binary(tmp, NO_SPEC_TOP_N_SCRIPT)
            caps = probe_usi_capabilities(binary, threads=1, spec_top_n=0, timeout_s=5)
            self.assertIn("Threads", caps["advertised_options"])
            self.assertNotIn("SpecTopN", caps["advertised_options"])


class RequireUsiCapabilitiesTests(unittest.TestCase):
    def test_full_support_binary_passes(self):
        with tempfile.TemporaryDirectory() as tmp:
            binary = _write_fake_binary(tmp, FULL_SUPPORT_SCRIPT)
            caps = require_usi_capabilities(binary, "base", 1, 0, 5)
            self.assertIn("SpecTopN", caps["advertised_options"])

    def test_missing_spec_top_n_exits_nonzero(self):
        with tempfile.TemporaryDirectory() as tmp:
            binary = _write_fake_binary(tmp, NO_SPEC_TOP_N_SCRIPT)
            with self.assertRaises(SystemExit) as ctx:
                require_usi_capabilities(binary, "candidate", 1, 0, 5)
            self.assertNotEqual(ctx.exception.code, 0)

    def test_missing_readyok_exits_nonzero(self):
        with tempfile.TemporaryDirectory() as tmp:
            binary = _write_fake_binary(tmp, NO_READYOK_SCRIPT)
            with self.assertRaises(SystemExit) as ctx:
                require_usi_capabilities(binary, "candidate", 1, 0, 5)
            self.assertNotEqual(ctx.exception.code, 0)


if __name__ == "__main__":
    unittest.main()
