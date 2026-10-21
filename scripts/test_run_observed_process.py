import errno
import fcntl
import json
from pathlib import Path
import signal
import subprocess
import time
import sys
import tempfile
import unittest
from unittest import mock
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

    def test_enospc_state_save_releases_physical_reserve_and_observes_one_child(self):
        for phase in ('starting', 'running', 'exited'):
            with self.subTest(phase=phase), tempfile.TemporaryDirectory() as td:
                root = Path(td)
                original_save = O.save
                injected = False

                def save_with_full_disk(path, document):
                    nonlocal injected
                    if document.get('status') == phase and not injected:
                        injected = True
                        reserve = root / '.observer-emergency-reserve'
                        self.assertEqual(reserve.stat().st_size, O.EmergencyReserve.size)
                        self.assertGreaterEqual(reserve.stat().st_blocks * 512,
                                                O.EmergencyReserve.size)
                        # Fail after the pending state has actually been written.
                        with mock.patch.object(O.os, 'fsync',
                                               side_effect=OSError(errno.ENOSPC, 'disk full')):
                            original_save(path, document)
                    else:
                        original_save(path, document)

                marker = root / 'executions'
                script = f"from pathlib import Path; p=Path({str(marker)!r}); p.open('a').write('once\\n'); print('finished')"
                with mock.patch.object(O, 'save', side_effect=save_with_full_disk):
                    result = self.run_child(script, root)
                self.assertTrue(injected)
                self.assertEqual(result['status'], 'exited')
                self.assertEqual(result['outcome']['returncode'], 0)
                self.assertEqual(result['persistence_recovery']['reason'], 'ENOSPC')
                self.assertEqual(marker.read_text().splitlines(), ['once'])
                self.assertFalse((root / '.observer-emergency-reserve').exists())
                self.assertEqual(json.loads((root / 'observer-status.json').read_text())['status'],
                                 'exited')

    def test_enospc_event_append_rolls_back_partial_line_and_preserves_exit(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            original_append = O.append
            injected = False

            def append_with_full_disk(path, document):
                nonlocal injected
                if document['event'] == 'child_exited' and not injected:
                    injected = True
                    with mock.patch.object(O.os, 'fsync',
                                           side_effect=OSError(errno.ENOSPC, 'disk full')):
                        original_append(path, document)
                else:
                    original_append(path, document)

            marker = root / 'executions'
            script = f"from pathlib import Path; Path({str(marker)!r}).open('a').write('once\\n')"
            with mock.patch.object(O, 'append', side_effect=append_with_full_disk):
                result = self.run_child(script, root)
            self.assertTrue(injected)
            self.assertEqual(result['outcome']['returncode'], 0)
            self.assertEqual(marker.read_text().splitlines(), ['once'])
            events = [json.loads(line) for line in
                      (root / 'observer-events.jsonl').read_text().splitlines()]
            exits = [event for event in events if event['event'] == 'child_exited']
            self.assertEqual(len(exits), 1)
            self.assertEqual(exits[0]['outcome']['returncode'], 0)
            self.assertEqual(exits[0]['persistence_recovery']['reason'], 'ENOSPC')
            saved = json.loads((root / 'observer-status.json').read_text())
            self.assertEqual(saved['persistence_recovery']['reason'], 'ENOSPC')
            self.assertFalse((root / '.observer-emergency-reserve').exists())

    def test_persistent_enospc_does_not_spin_or_relaunch_child(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            original_save = O.save
            failures = 0

            def save_with_full_disk(path, document):
                nonlocal failures
                if document['status'] in {'running', 'observer_error'}:
                    failures += 1
                    raise OSError(errno.ENOSPC, 'disk remains full')
                original_save(path, document)

            with mock.patch.object(O, 'save', side_effect=save_with_full_disk):
                with self.assertRaises(OSError):
                    self.run_child('import time; time.sleep(30)', root)
            self.assertEqual(failures, 3)  # failed write, one reserve retry, final status.
            events = [json.loads(line) for line in
                      (root / 'observer-events.jsonl').read_text().splitlines()]
            self.assertEqual(sum(event['event'] == 'attempt_starting' for event in events), 1)
            failure = next(event for event in events if event['event'] == 'observer_error')
            self.assertIsNotNone(failure['child_returncode'])
            self.assertFalse((root / '.observer-emergency-reserve').exists())

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
