#!/usr/bin/env python3
"""Small process fixtures for cleanup after monitor failure, without engines."""
import errno
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import run_observed_process as observer


@unittest.skipUnless(os.name == "posix", "POSIX process-group ownership")
class ManagedChildTest(unittest.TestCase):
    def spawn(self, program):
        return observer.managed_child(
            [sys.executable, "-c", program], stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            stop_grace_seconds=.2, kill_grace_seconds=2)

    def test_disk_probe_or_telemetry_failure_reaps_child_without_retry(self):
        for failure in (FileNotFoundError("archive unplugged"),
                        OSError(errno.ENOSPC, "telemetry disk full"), KeyboardInterrupt()):
            with self.subTest(failure=type(failure).__name__):
                with self.assertRaises(type(failure)) as caught:
                    with self.spawn("import time; time.sleep(30)") as child:
                        raise failure
                self.assertIs(caught.exception, failure)
                self.assertIsNotNone(child.returncode)
                self.assertFalse(failure.cleanup_incomplete)

    def test_normal_completed_child_is_not_signalled(self):
        with mock.patch.object(observer.os, "killpg") as signal_group:
            with self.spawn("pass") as child:
                self.assertEqual(child.wait(timeout=5), 0)
            signal_group.assert_not_called()
        self.assertEqual(child.returncode, 0)

    def test_clean_early_scope_exit_stops_child(self):
        with self.spawn("import time; time.sleep(30)") as child:
            pass
        self.assertIsNotNone(child.returncode)

    def test_sigterm_ignoring_child_is_killed_within_budget(self):
        with tempfile.TemporaryDirectory() as directory:
            ready = Path(directory) / "ready"
            program = ("import signal,time; from pathlib import Path; "
                       "signal.signal(signal.SIGTERM,signal.SIG_IGN); "
                       f"Path({str(ready)!r}).touch(); time.sleep(30)")
            with self.spawn(program) as child:
                deadline = time.monotonic() + 5
                while not ready.exists() and time.monotonic() < deadline:
                    time.sleep(.01)
                self.assertTrue(ready.exists())
            self.assertEqual(child.returncode, -signal.SIGKILL)

    def test_cleanup_failure_keeps_original_monitor_error(self):
        original = OSError("archive disappeared")
        child = mock.Mock(pid=123, returncode=None)
        child.poll.return_value = None
        report = {"signals": [], "returncode": None,
                  "errors": [{"action": "wait", "error_type": "TimeoutExpired"}]}
        with mock.patch.object(observer.subprocess, "Popen", return_value=child) as spawn, \
                mock.patch.object(observer, "cleanup_child", side_effect=observer.ChildCleanupError(report)):
            with self.assertRaises(OSError) as caught:
                with observer.managed_child(["fixture"]):
                    raise original
        self.assertIs(caught.exception, original)
        self.assertTrue(original.cleanup_incomplete)
        self.assertEqual(original.cleanup_errors, report["errors"])
        spawn.assert_called_once()

    def test_clean_scope_cleanup_failure_is_explicitly_incomplete(self):
        child = mock.Mock(pid=123, returncode=None)
        failure = observer.ChildCleanupError({"signals": [], "errors": [], "returncode": None})
        with mock.patch.object(observer.subprocess, "Popen", return_value=child), \
                mock.patch.object(observer, "cleanup_child", side_effect=failure):
            with self.assertRaises(observer.ChildCleanupError) as caught:
                with observer.managed_child(["fixture"]):
                    pass
        self.assertIs(caught.exception, failure)
        self.assertTrue(failure.cleanup_incomplete)

    def test_invalid_ownership_or_budget_is_rejected_before_spawn(self):
        for options in ({"start_new_session": False}, {"start_new_session": True},
                        {"process_group": 0}, {"preexec_fn": lambda: None},
                        {"stdout": subprocess.PIPE}, {"stdin": subprocess.PIPE}, {"stderr": subprocess.PIPE},
                        {"stop_grace_seconds": 0}, {"kill_grace_seconds": float("nan")}):
            with mock.patch.object(observer.subprocess, "Popen") as spawn:
                with self.assertRaises(ValueError):
                    with observer.managed_child(["fixture"], **options):
                        self.fail("invalid context was entered")
                spawn.assert_not_called()

    def test_non_posix_rejected_before_spawn(self):
        with mock.patch.object(observer.os, "name", "nt"), mock.patch.object(observer.subprocess, "Popen") as spawn:
            with self.assertRaises(ValueError):
                with observer.managed_child(["fixture"]):
                    self.fail("non-POSIX context entered")
            spawn.assert_not_called()

    def test_interrupted_cleanup_does_not_replace_monitor_failure(self):
        original = OSError("archive disappeared")
        child = mock.Mock(pid=123, returncode=None)
        with mock.patch.object(observer.subprocess, "Popen", return_value=child), \
                mock.patch.object(observer, "cleanup_child", side_effect=KeyboardInterrupt()):
            with self.assertRaises(OSError) as caught:
                with observer.managed_child(["fixture"]):
                    raise original
        self.assertIs(caught.exception, original)
        self.assertTrue(original.cleanup_incomplete)
        self.assertEqual(original.cleanup_errors[0]["error_type"], "KeyboardInterrupt")

    def test_caller_owned_output_file_remains_open(self):
        with tempfile.TemporaryFile() as output:
            with observer.managed_child([sys.executable, "-c", "print(123)"], stdout=output) as child:
                self.assertEqual(child.wait(timeout=5), 0)
            self.assertFalse(output.closed)
            output.seek(0)
            self.assertEqual(output.read(), b"123\n")

    def test_spawn_error_is_preserved_without_cleanup_or_retry(self):
        with mock.patch.object(observer.subprocess, "Popen", side_effect=FileNotFoundError("missing")) as spawn, \
                mock.patch.object(observer, "cleanup_child") as cleanup:
            with self.assertRaises(FileNotFoundError):
                with observer.managed_child(["fixture"]):
                    self.fail("spawn should fail")
            spawn.assert_called_once()
            cleanup.assert_not_called()


if __name__ == "__main__":
    unittest.main()
