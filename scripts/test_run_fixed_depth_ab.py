#!/usr/bin/env python3
"""Unit tests for run_fixed_depth_ab.py's USI capability guard and
interactive protocol driver.

Motivation: run 31362228815 silently compared base (SpecTopN USI option
present) against a candidate binary built from a commit that predates the
option -- `setoption name SpecTopN value 0` was accepted-and-ignored by the
candidate rather than rejected, so the two sides actually ran at different
SpecTopN values, producing a bogus 238x node-count outlier unrelated to the
change under test. probe_usi_capabilities()/require_usi_capabilities() exist
to catch that class of silent mismatch before a corpus run starts.

Run 31363151597 then hit a second, independent bug: `go` is asynchronous in
this engine (spawns a search thread, the main USI loop returns immediately),
and the driver used to send its whole command script -- including `quit`
right after `go depth N` -- as one string via `subprocess.run(input=...)`.
The main loop read `quit` and aborted the in-flight search before it had a
chance to produce a bestmove on any position whose search didn't happen to
finish first, printing `bestmove resign` instead. InteractiveDriverTests
pins the fix: `run_one_position` must wait for an actual `bestmove` line
before ever sending `quit`.

Each test drives a tiny fake "engine" (a shell or Python script standing in
for the real binary) through the real subprocess-based driver -- no cargo
build, no real engine, no network. Matching this repo's
scripts/test_gate_resource_preflight.py convention: stdlib unittest only.

Run: python3 scripts/test_run_fixed_depth_ab.py
"""
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path

from run_fixed_depth_ab import (
    _classify,
    _status,
    probe_usi_capabilities,
    require_usi_capabilities,
    run_one_position,
)

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


# Models the real engine's asynchronous `go`: a `go depth N` sets a
# "searching" flag and returns to the command loop immediately; the
# "search" only actually completes GO_DELAY_S later (checked via a
# select() timeout, so this stays responsive to stdin the whole time --
# the same shape as the real engine's search thread being interruptible
# by `stop`/`quit` mid-search). Any `stop` or `quit` received while still
# "searching" prints `bestmove resign` immediately, exactly like the real
# engine's abort_and_join_inflight_search() path.
FAKE_ASYNC_ENGINE_SCRIPT = """#!/usr/bin/env python3
import sys, select, time, os

GO_DELAY_S = 0.2

def out(s):
    sys.stdout.write(s + "\\n")
    sys.stdout.flush()

# Raw os.read on the fd, not sys.stdin.readline(): a buffered TextIOWrapper
# can pull more bytes than one line into its own userspace buffer on a
# single read, invisible to select() -- select() then reports the fd as
# NOT ready (nothing new at the OS level) even though a full line is
# already sitting in Python's buffer, and this loop would block forever
# on select() waiting for bytes that already arrived. Reading the fd
# directly keeps select() and the buffer consistent.
fd = sys.stdin.fileno()
buf = b""

def next_line():
    global buf
    if b"\\n" in buf:
        line, buf = buf.split(b"\\n", 1)
        return line.decode()
    return None

searching_since = None
while True:
    line = next_line()
    if line is None:
        timeout = None
        if searching_since is not None:
            timeout = max(0.0, GO_DELAY_S - (time.monotonic() - searching_since))
        ready, _, _ = select.select([fd], [], [], timeout)
        if ready:
            chunk = os.read(fd, 4096)
            if chunk == b"":
                break
            buf += chunk
        elif searching_since is not None:
            out("info depth 9 score cp 42 nodes 12345 pv 7g7f")
            out("bestmove 7g7f")
            searching_since = None
        continue
    cmd = line.strip()
    if cmd == "usi":
        out("id name fake-async-engine")
        out("option name Threads type spin default 1 min 1 max 512")
        out("option name SpecTopN type spin default 3 min 0 max 512")
        out("usiok")
    elif cmd == "isready":
        out("readyok")
    elif cmd.startswith("position"):
        pass
    elif cmd.startswith("go"):
        searching_since = time.monotonic()
    elif cmd == "stop":
        if searching_since is not None:
            out("bestmove resign")
            searching_since = None
    elif cmd == "quit":
        if searching_since is not None:
            out("bestmove resign")
            searching_since = None
        break
"""


def _write_fake_async_engine(tmpdir):
    path = Path(tmpdir) / "fake-async-engine.py"
    path.write_text(FAKE_ASYNC_ENGINE_SCRIPT)
    path.chmod(path.stat().st_mode | stat.S_IEXEC)
    return path


class InteractiveDriverTests(unittest.TestCase):
    def test_old_style_all_at_once_input_reproduces_the_resign_race(self):
        """Sanity check that the fake engine actually models the bug: the
        OLD driver's approach (whole script, quit queued right behind go,
        via subprocess.run(input=...)) must still reproduce `bestmove
        resign` against this fake engine, or this test proves nothing."""
        with tempfile.TemporaryDirectory() as tmp:
            binary = _write_fake_async_engine(tmp)
            stdin_text = (
                "usi\n"
                "setoption name Threads value 1\n"
                "setoption name SpecTopN value 0\n"
                "isready\n"
                "position startpos\n"
                "go depth 9\n"
                "quit\n"
            )
            proc = subprocess.run(
                [str(binary)], input=stdin_text, capture_output=True, text=True, timeout=5
            )
            self.assertIn("bestmove resign", proc.stdout)

    def test_interactive_driver_waits_for_bestmove_before_quitting(self):
        """The actual regression pin: run_one_position must not send quit
        until it has observed a real bestmove line, so it gets the
        engine's actual search result instead of an abort-induced
        resign."""
        with tempfile.TemporaryDirectory() as tmp:
            binary = _write_fake_async_engine(tmp)
            entry = {"id": "async-smoke", "category": "smoke"}
            result = run_one_position(
                binary, entry, depth=9, threads=1, spec_top_n=0, timeout_s=5
            )
            self.assertEqual(result["bestmove"], "7g7f")
            self.assertEqual(result["depth_reached"], 9)
            self.assertEqual(result["nodes"], 12345)
            self.assertFalse(result["unexpected_resign"])
            self.assertFalse(result["incomplete_output"])
            self.assertFalse(result["timed_out"])
            self.assertFalse(result["panicked"])
            self.assertEqual(_status(result), "ok")


class ClassifyResignTests(unittest.TestCase):
    def test_unexpected_resign_is_a_correctness_failure_not_a_bestmove(self):
        result = {
            "bestmove": "resign",
            "depth_reached": None,
            "nodes": None,
            "timed_out": False,
            "panicked": False,
            "illegal_move": False,
            "unexpected_resign": False,
            "incomplete_output": False,
        }
        _classify(result, allow_resign=False)
        self.assertTrue(result["unexpected_resign"])
        self.assertEqual(_status(result), "unexpected_resign")

    def test_allow_resign_true_does_not_flag_a_resign_bestmove(self):
        result = {
            "bestmove": "resign",
            "depth_reached": None,
            "nodes": None,
            "timed_out": False,
            "panicked": False,
            "illegal_move": False,
            "unexpected_resign": False,
            "incomplete_output": False,
        }
        _classify(result, allow_resign=True)
        self.assertFalse(result["unexpected_resign"])
        self.assertEqual(_status(result), "ok")

    def test_missing_depth_on_a_real_move_is_incomplete_output(self):
        result = {
            "bestmove": "7g7f",
            "depth_reached": None,
            "nodes": None,
            "timed_out": False,
            "panicked": False,
            "illegal_move": False,
            "unexpected_resign": False,
            "incomplete_output": False,
        }
        _classify(result, allow_resign=False)
        self.assertTrue(result["incomplete_output"])
        self.assertEqual(_status(result), "incomplete_output")


if __name__ == "__main__":
    unittest.main()
