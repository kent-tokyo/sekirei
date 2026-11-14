#!/usr/bin/env python3
"""End-to-end protocol checks for the opt-in local r6 PGO launcher."""
from __future__ import annotations

import hashlib
import os
import select
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("run_local_r6_pgo.py")
FAKE = """#!/usr/bin/env python3
import os
import sys
settings = {}
for raw in sys.stdin:
    line = raw.strip()
    if line == 'usi':
        print('option name FV_SCALE type spin default 16 min 1 max 128', flush=True)
        print('option name EvalFile type string default ', flush=True)
        print('usiok', flush=True)
    elif line.startswith('setoption name '):
        name, _, value = line.removeprefix('setoption name ').partition(' value ')
        settings[name] = value
        if name == 'FV_SCALE':
            shown = '16' if os.environ.get('FAKE_BAD_SCALE') == '1' else value
            print('info string FV_SCALE ' + shown, flush=True)
        if name == 'NnueOutput': print('info string NNUE output mode ' + value, flush=True)
    elif line == 'isready':
        if os.environ.get('FAKE_MISSING_LOAD') != '1':
            print('info string NNUE weights loaded from ' + settings.get('EvalFile', ''), flush=True)
        print('readyok', flush=True)
    elif line.startswith('go '):
        print('bestmove 7g7f', flush=True)
    elif line == 'quit':
        with open(os.environ['FAKE_SETTINGS'], 'w') as output:
            output.write(repr(settings))
        break
"""


class LauncherTests(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        root = Path(self.directory.name)
        self.engine = root / "fake-engine"
        self.engine.write_text(FAKE)
        self.engine.chmod(0o755)
        self.weights = root / "r6.bin"
        self.weights.write_bytes(b"private test weight")
        self.settings = root / "settings.txt"

    def command(self) -> list[str]:
        return [sys.executable, str(SCRIPT), "--engine", str(self.engine),
                "--engine-sha256", hashlib.sha256(self.engine.read_bytes()).hexdigest(),
                "--weights", str(self.weights),
                "--weights-sha256", hashlib.sha256(self.weights.read_bytes()).hexdigest()]

    def invoke(self, commands: str, *, missing_load: bool = False,
               bad_scale: bool = False) -> subprocess.CompletedProcess[str]:
        env = os.environ.copy()
        env["FAKE_SETTINGS"] = str(self.settings)
        if missing_load:
            env["FAKE_MISSING_LOAD"] = "1"
        if bad_scale:
            env["FAKE_BAD_SCALE"] = "1"
        return subprocess.run(self.command(), input=commands, text=True, capture_output=True,
                              env=env, timeout=5)

    def test_pins_r6_settings_before_ready_and_go(self) -> None:
        env = os.environ.copy()
        env["FAKE_SETTINGS"] = str(self.settings)
        child = subprocess.Popen(self.command(), stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                 stderr=subprocess.PIPE, bufsize=0, env=env)
        assert child.stdin is not None and child.stdout is not None
        def read_until(expected: str) -> str:
            output = b""
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline:
                if not select.select([child.stdout], [], [], deadline - time.monotonic())[0]:
                    break
                output += os.read(child.stdout.fileno(), 65536)
                if expected.encode() in output:
                    return output.decode()
            self.fail(f"missing {expected}: {output!r}")
        try:
            child.stdin.write(b"usi\nsetoption name FV_SCALE value 16\n"
                              b"setoption name EvalFile value wrong.bin\nisready\n")
            child.stdin.flush()
            output = read_until("readyok")
            self.assertIn("option name FV_SCALE type spin default 24", output)
            child.stdin.write(b"position startpos\ngo nodes 1000\n")
            child.stdin.flush()
            output += read_until("bestmove")
            child.stdin.write(b"quit\n")
            child.stdin.flush()
            self.assertEqual(child.wait(timeout=5), 0, child.stderr.read().decode())
            self.assertIn("bestmove 7g7f", output)
        finally:
            if child.poll() is None:
                child.kill()
                child.wait()
            for stream in (child.stdin, child.stdout, child.stderr):
                if stream is not None:
                    stream.close()
        settings = self.settings.read_text()
        self.assertIn("'FV_SCALE': '24'", settings)
        self.assertIn(f"'EvalFile': '{self.weights.resolve()}'", settings)
        self.assertIn("'Threads': '1'", settings)
        self.assertIn("'SpecTopN': '0'", settings)

    def test_readyok_without_load_ack_is_not_forwarded(self) -> None:
        result = self.invoke("usi\nisready\n", missing_load=True)
        self.assertEqual(result.returncode, 2)
        self.assertNotIn("readyok", result.stdout)
        self.assertIn("without r6 load", result.stderr)

    def test_readyok_with_wrong_scale_is_not_forwarded(self) -> None:
        result = self.invoke("usi\nisready\n", bad_scale=True)
        self.assertEqual(result.returncode, 2)
        self.assertNotIn("readyok", result.stdout)
        self.assertIn("without r6 load, scale", result.stderr)

    def test_changed_weight_is_rejected_before_engine_start(self) -> None:
        command = self.command()
        self.weights.write_bytes(b"changed")
        result = subprocess.run(command, input="usi\n", text=True, capture_output=True, timeout=5)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("SHA-256 mismatched", result.stderr)


if __name__ == "__main__":
    unittest.main()
