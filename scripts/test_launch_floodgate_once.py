import json
import os
import plistlib
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
LAUNCHER = ROOT / "scripts/launch_floodgate_once.py"


FAKE_LAUNCHCTL = r'''#!/usr/bin/env python3
import json
import os
import sys
from pathlib import Path

state_path = Path(os.environ["FAKE_LAUNCHD_STATE"])
log_path = Path(os.environ["FAKE_LAUNCHD_LOG"])
with log_path.open("a", encoding="utf-8") as handle:
    handle.write(json.dumps(sys.argv[1:]) + "\n")

command = sys.argv[1]
if command == "print":
    if not state_path.exists():
        print("Could not find service", file=sys.stderr)
        raise SystemExit(113)
    state = json.loads(state_path.read_text(encoding="utf-8"))
    print(f'\truns = {state["runs"]}')
    print(f'\tstate = "{state.get("state", "running")}"')
elif command == "bootstrap":
    state_path.write_text(
        json.dumps({"runs": int(os.environ["FAKE_BOOTSTRAP_RUNS"]), "state": "running"}),
        encoding="utf-8",
    )
elif command == "kickstart":
    state = json.loads(state_path.read_text(encoding="utf-8"))
    state["runs"] += 1
    state["state"] = "running"
    state_path.write_text(json.dumps(state), encoding="utf-8")
else:
    print(f"unexpected command: {command}", file=sys.stderr)
    raise SystemExit(2)
'''


class FloodgateLaunchTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        self.runtime = self.root / "runtime"
        self.state = self.root / "state.json"
        self.log = self.root / "launchctl.jsonl"
        self.launchctl = self.root / "launchctl"
        self.launchctl.write_text(FAKE_LAUNCHCTL, encoding="utf-8")
        self.launchctl.chmod(0o755)

    def tearDown(self):
        self.temporary.cleanup()

    def plist(self, run_at_load=True, max_games="5"):
        path = self.root / f"agent-{run_at_load}-{max_games}.plist"
        path.write_bytes(
            plistlib.dumps(
                {
                    "Label": "com.kent-tokyo.sekirei-floodgate.test",
                    "ProgramArguments": [
                        "python3",
                        "scripts/floodgate_supervisor.py",
                        "--state-dir",
                        str(self.runtime),
                        "--one-shot",
                        "--",
                        "sekirei-csa",
                        "--loop",
                        "--max-games",
                        max_games,
                    ],
                    "RunAtLoad": run_at_load,
                    "KeepAlive": False,
                }
            )
        )
        return path

    def invoke(self, plist, bootstrap_runs, runtime=None):
        environment = os.environ.copy()
        environment.update(
            {
                "FAKE_LAUNCHD_STATE": str(self.state),
                "FAKE_LAUNCHD_LOG": str(self.log),
                "FAKE_BOOTSTRAP_RUNS": str(bootstrap_runs),
            }
        )
        return subprocess.run(
            [
                sys.executable,
                str(LAUNCHER),
                "--plist",
                str(plist),
                "--runtime-dir",
                str(runtime or self.runtime),
                "--domain",
                "gui/501",
                "--launchctl",
                str(self.launchctl),
            ],
            capture_output=True,
            text=True,
            check=False,
            env=environment,
        )

    def commands(self):
        return [json.loads(line) for line in self.log.read_text(encoding="utf-8").splitlines()]

    def test_run_at_load_uses_bootstrap_only_and_records_one_run(self):
        result = self.invoke(self.plist(run_at_load=True), bootstrap_runs=1)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(f"runtime={self.runtime.resolve()}", result.stdout)
        self.assertEqual([command[0] for command in self.commands()], ["print", "bootstrap", "print"])
        receipt = json.loads((self.runtime / "launchd-start.json").read_text(encoding="utf-8"))
        self.assertTrue(receipt["accepted"])
        self.assertEqual(receipt["launchd_runs"], 1)
        self.assertEqual(receipt["max_games"], 5)
        self.assertEqual(receipt["commands"], ["bootstrap"])

    def test_no_run_at_load_uses_one_non_forcing_kickstart(self):
        result = self.invoke(self.plist(run_at_load=False), bootstrap_runs=0)
        self.assertEqual(result.returncode, 0, result.stderr)
        commands = self.commands()
        self.assertEqual([command[0] for command in commands], ["print", "bootstrap", "kickstart", "print"])
        self.assertNotIn("-k", commands[2])
        receipt = json.loads((self.runtime / "launchd-start.json").read_text(encoding="utf-8"))
        self.assertEqual(receipt["commands"], ["bootstrap", "kickstart"])

    def test_registered_label_is_refused_and_runtime_is_reported(self):
        self.state.write_text(json.dumps({"runs": 1, "state": "running"}), encoding="utf-8")
        result = self.invoke(self.plist(), bootstrap_runs=1)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("already registered", result.stderr)
        self.assertIn(f"runtime={self.runtime.resolve()}", result.stderr)
        self.assertEqual([command[0] for command in self.commands()], ["print"])

    def test_duplicate_launch_count_is_recorded_and_rejected_before_acceptance(self):
        result = self.invoke(self.plist(run_at_load=True), bootstrap_runs=2)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("runs != 1", result.stderr)
        receipt = json.loads((self.runtime / "launchd-start.json").read_text(encoding="utf-8"))
        self.assertFalse(receipt["accepted"])
        self.assertEqual(receipt["launchd_runs"], 2)

    def test_unbounded_or_keepalive_plist_is_rejected_without_launchctl(self):
        unbounded = self.plist(max_games="0")
        result = self.invoke(unbounded, bootstrap_runs=1)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("--max-games > 0", result.stderr)
        self.assertFalse(self.log.exists())

        document = plistlib.loads(unbounded.read_bytes())
        document["KeepAlive"] = True
        unbounded.write_bytes(plistlib.dumps(document))
        result = self.invoke(unbounded, bootstrap_runs=1, runtime=self.root / "second-runtime")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("KeepAlive=false", result.stderr)
        self.assertFalse(self.log.exists())

    def test_runtime_directory_must_match_the_rendered_plist(self):
        result = self.invoke(self.plist(), bootstrap_runs=1, runtime=self.root / "wrong-runtime")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("must match", result.stderr)
        self.assertIn(str(self.runtime.resolve()), result.stderr)
        self.assertFalse(self.log.exists())


if __name__ == "__main__":
    unittest.main()
