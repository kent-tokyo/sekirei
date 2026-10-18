import fcntl
import json
from pathlib import Path
import signal
import subprocess
import time
import sys
import tempfile
import unittest
from scripts import run_observed_process as O


class ObservedProcessTests(unittest.TestCase):
    def run_child(self, script, root):
        return O.observe([sys.executable, "-c", script], root, poll_seconds=.05)

    def test_normal_exit_preserves_stdout_and_durable_event(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            result = self.run_child("print('finished')", root)
            self.assertEqual(result['outcome']['kind'], 'normal_exit')
            self.assertEqual(Path(result['stdout']).read_text().strip(), 'finished')
            events = [json.loads(x) for x in (root/'observer-events.jsonl').read_text().splitlines()]
            self.assertEqual(events[-1]['event'], 'child_exited')
            self.assertEqual(events[-1]['outcome']['returncode'], 0)
            self.assertEqual(json.loads((root/'observer-status.json').read_text())['status'], 'exited')

    def test_nonzero_exit_preserves_stderr_and_code(self):
        with tempfile.TemporaryDirectory() as td:
            result = self.run_child("import sys;print('failure detail',file=sys.stderr);sys.exit(7)", Path(td))
            self.assertEqual(result['outcome']['returncode'], 7)
            self.assertIn('failure detail', Path(result['stderr']).read_text())

    def test_killed_child_leaves_signal_record_without_inventing_oom(self):
        with tempfile.TemporaryDirectory() as td:
            result = self.run_child("import os,signal;os.kill(os.getpid(),signal.SIGKILL)", Path(td))
            self.assertEqual(result['outcome']['returncode'], -signal.SIGKILL)
            self.assertEqual(result['outcome']['signal_name'], 'SIGKILL')
            self.assertTrue(result['outcome']['underlying_cause'].startswith('unknown'))
            saved = json.loads((Path(td)/'observer-status.json').read_text())
            self.assertEqual(saved['outcome'], result['outcome'])

    def test_requested_sigterm_is_observed_by_outer_process(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            command = [sys.executable, str(Path(O.__file__)), "--state-dir", td,
                       "--poll-seconds", ".05", "--", sys.executable, "-c",
                       "import time;time.sleep(30)"]
            process = subprocess.Popen(command, stdout=subprocess.DEVNULL,
                                       stderr=subprocess.DEVNULL)
            try:
                deadline = time.monotonic() + 5
                while time.monotonic() < deadline:
                    file = root/'observer-status.json'
                    if file.exists() and json.loads(file.read_text()).get('status') == 'running':
                        break
                    time.sleep(.01)
                else:
                    self.fail('observer did not launch child')
                process.send_signal(signal.SIGTERM)
                process.wait(timeout=5)
                result = json.loads((root/'observer-status.json').read_text())
                self.assertEqual(result['outcome']['stop_requested_signal'], signal.SIGTERM)
                self.assertEqual(result['outcome']['signal_name'], 'SIGTERM')
                self.assertEqual(result['status'], 'exited')
            finally:
                if process.poll() is None:
                    process.terminate()
                    process.wait(timeout=5)

    def test_nonfinite_polling_is_rejected_before_launch(self):
        with tempfile.TemporaryDirectory() as td:
            for value in [float('nan'), float('inf'), 0, -1]:
                with self.subTest(value=value), self.assertRaises(ValueError):
                    O.observe([sys.executable, '-c', 'pass'], Path(td), value)

    def test_attempt_logs_are_not_overwritten(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            first = self.run_child("print('one')", root)
            second = self.run_child("print('two')", root)
            self.assertEqual(second['attempt'], 2)
            self.assertEqual(Path(first['stdout']).read_text().strip(), 'one')
            self.assertEqual(Path(second['stdout']).read_text().strip(), 'two')

    def test_lock_refuses_duplicate_observer(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            with (root/'observer.lock').open('a') as lock:
                fcntl.flock(lock, fcntl.LOCK_EX|fcntl.LOCK_NB)
                with self.assertRaises(RuntimeError):
                    self.run_child("print('must not run')", root)
            self.assertFalse((root/'attempt-0001.stdout').exists())

    def test_launch_failure_is_saved(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            with self.assertRaises(FileNotFoundError):
                O.observe(['/nonexistent/sekirei-observer-test'], root)
            state = json.loads((root/'observer-status.json').read_text())
            self.assertEqual(state['error']['error_type'], 'FileNotFoundError')
            self.assertEqual(state['status'], 'observer_error')

    def test_unclosed_starting_state_refuses_ambiguous_spawn(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            previous = {'status':'starting','attempt':1}
            (root/'observer-status.json').write_text(json.dumps(previous))
            with self.assertRaises(RuntimeError):
                self.run_child("print('must not launch')", root)
            self.assertFalse((root/'attempt-0002.stdout').exists())
            self.assertEqual(json.loads((root/'observer-status.json').read_text()), previous)

    def test_previous_live_direct_child_refuses_duplicate(self):
        import os
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            (root/'observer-status.json').write_text(json.dumps({'status':'running','attempt':1,'child_pid':os.getpid()}))
            with self.assertRaises(RuntimeError):
                self.run_child("print('must not launch')", root)
            self.assertFalse((root/'attempt-0002.stdout').exists())

    def test_unknown_previous_observer_exit_is_not_fabricated(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            (root/'observer-status.json').write_text(json.dumps({'status':'running','attempt':1,'child_pid':99999999}))
            self.run_child("pass", root)
            events = [json.loads(x) for x in (root/'observer-events.jsonl').read_text().splitlines()]
            old = next(x for x in events if x['event']=='previous_observer_disappeared')
            self.assertTrue(old['cause'].startswith('unknown'))


if __name__ == '__main__':
    unittest.main()
