import errno
import fcntl
import json
import os
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

    def test_second_enospc_preserves_terminal_record_without_relaunch(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            original_save = O.save
            initial_failure = False
            second_failure = False
            marker = root / 'executions'
            script = (f"from pathlib import Path; Path({str(marker)!r}).open('a').write('once\\n'); "
                      "import time; time.sleep(30)")

            def save_with_repeated_full_disk(path, document):
                nonlocal initial_failure, second_failure
                terminal = root / '.observer-terminal-reserve'
                if document['status'] == 'running':
                    self.assertTrue(terminal.exists())
                    self.assertGreaterEqual(terminal.stat().st_blocks * 512,
                                            O.EmergencyReserve.size)
                    if not initial_failure:
                        initial_failure = True
                        raise OSError(errno.ENOSPC, 'first disk exhaustion')
                    if 'last_heartbeat' in document:
                        second_failure = True
                        raise OSError(errno.ENOSPC, 'second disk exhaustion')
                if document['status'] == 'observer_error':
                    self.assertFalse(terminal.exists())
                original_save(path, document)

            with mock.patch.object(O, 'save', side_effect=save_with_repeated_full_disk):
                with self.assertRaisesRegex(OSError, 'second disk exhaustion'):
                    self.run_child(script, root)
            self.assertTrue(initial_failure and second_failure)
            self.assertEqual(marker.read_text().splitlines(), ['once'])
            saved = json.loads((root / 'observer-status.json').read_text())
            self.assertEqual(saved['status'], 'observer_error')
            self.assertIn('second disk exhaustion', saved['error']['error'])
            self.assertIsNotNone(saved['error']['child_returncode'])
            self.assertEqual(saved['error']['observer_cleanup_signal'], 'SIGTERM')
            self.assertEqual(saved['terminal_journal_reserve']['released_bytes'],
                             O.EmergencyReserve.size)
            events = [json.loads(x) for x in (root / 'observer-events.jsonl').read_text().splitlines()]
            self.assertEqual(sum(x['event'] == 'child_started' for x in events), 1)
            self.assertEqual(events[-1]['event'], 'observer_error')
            self.assertFalse((root / '.observer-emergency-reserve').exists())
            self.assertFalse((root / '.observer-terminal-reserve').exists())

    def test_terminal_unlink_failure_preserves_primary_error_and_unlocks(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            original_save = O.save
            close = O.EmergencyReserve.close

            def fail_running_save(path, document):
                if document['status'] == 'running':
                    raise OSError(errno.EIO, 'primary journal failure')
                original_save(path, document)

            def fail_terminal_close(reserve):
                if reserve.path.name == '.observer-terminal-reserve':
                    raise OSError(errno.ENOSPC, 'terminal unlink failure')
                close(reserve)

            with mock.patch.object(O, 'save', side_effect=fail_running_save), \
                 mock.patch.object(O.EmergencyReserve, 'close', fail_terminal_close):
                with self.assertRaisesRegex(OSError, 'primary journal failure'):
                    self.run_child('import time; time.sleep(30)', root)
            saved = json.loads((root / 'observer-status.json').read_text())
            self.assertIn('primary journal failure', saved['error']['error'])
            self.assertIn('terminal unlink failure',
                          saved['error']['terminal_reserve_release_error'])
            with (root / 'observer.lock').open('a') as lock:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)

    def test_ordinary_reserve_unlink_failure_keeps_original_enospc(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            reserve = O.EmergencyReserve(root)
            reserve.allocate()

            def fail_write(_path, _document):
                raise OSError(errno.ENOSPC, 'original journal full')

            with mock.patch.object(reserve, 'close',
                                   side_effect=OSError(errno.EIO, 'reserve unlink failed')):
                with self.assertRaisesRegex(OSError, 'original journal full') as caught:
                    reserve.write(fail_write, root / 'observer-status.json', {})
            self.assertEqual(caught.exception.errno, errno.ENOSPC)
            reserve.close()

    def test_child_exit_during_cleanup_escalation_still_persists_primary_error(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            original_save = O.save
            child = mock.Mock(pid=424242, returncode=None)
            child.poll.side_effect = lambda: child.returncode
            waits = 0

            def wait_after_grace(**kwargs):
                nonlocal waits
                waits += 1
                if waits == 1:
                    raise subprocess.TimeoutExpired('synthetic child', 1)
                child.returncode = 0
                return 0

            child.wait.side_effect = wait_after_grace

            def fail_running_save(path, document):
                if document['status'] == 'running':
                    raise OSError(errno.EIO, 'primary journal failure')
                original_save(path, document)

            def signal_race(_pid, number):
                if number == signal.SIGKILL:
                    child.returncode = 0
                    raise ProcessLookupError()

            with mock.patch.object(O, 'snapshot', return_value={}), \
                 mock.patch.object(O, 'save', side_effect=fail_running_save), \
                 mock.patch.object(O.subprocess, 'Popen', return_value=child) as spawn, \
                 mock.patch.object(O.os, 'killpg', side_effect=signal_race):
                with self.assertRaisesRegex(OSError, 'primary journal failure'):
                    self.run_child('synthetic child', root)
                spawn.assert_called_once()
            saved = json.loads((root / 'observer-status.json').read_text())
            self.assertEqual(saved['status'], 'observer_error')
            self.assertEqual(saved['error']['child_returncode'], 0)
            self.assertEqual(saved['error']['observer_cleanup_signal'], 'SIGTERM')
            self.assertIn('primary journal failure', saved['error']['error'])
            self.assertFalse((root / '.observer-terminal-reserve').exists())

    def test_cont_permission_exit_race_is_recorded_without_signalling_reaped_pid(self):
        child = mock.Mock(pid=424242, returncode=None)
        child.poll.side_effect = lambda: child.returncode

        def exit_before_cont(_pid, number):
            if number == signal.SIGCONT:
                child.returncode = -signal.SIGTERM
                raise PermissionError(errno.EPERM, 'exited group')

        with mock.patch.object(O.os, 'killpg', side_effect=exit_before_cont) as killpg:
            report = O.cleanup_child(child, .01, .01)
        self.assertEqual(report['returncode'], -signal.SIGTERM)
        self.assertEqual(report['errors'][0]['action'], 'killpg:SIGCONT')
        child.send_signal.assert_not_called()
        child.wait.assert_not_called()
        self.assertEqual(killpg.call_count, 2)

    def test_cleanup_invalid_budgets_are_rejected_before_child_access(self):
        for values in ((float('inf'), 1), (1, float('nan')), (0, 1), (1, -1)):
            with self.subTest(values=values):
                child = mock.Mock()
                with self.assertRaises(ValueError):
                    O.cleanup_child(child, *values)
                child.poll.assert_not_called()

    def test_already_reaped_child_is_never_signalled_again(self):
        child = mock.Mock(pid=424242, returncode=0)
        child.poll.return_value = 0
        with mock.patch.object(O.os, 'killpg') as killpg:
            report = O.cleanup_child(child)
        self.assertEqual(report, {'signals': [], 'errors': [], 'returncode': 0})
        killpg.assert_not_called()
        child.send_signal.assert_not_called()
        child.wait.assert_not_called()

    def test_live_permission_denial_is_incomplete_and_waits_are_bounded(self):
        child = mock.Mock(pid=424242, returncode=None)
        child.poll.return_value = None
        child.send_signal.side_effect = PermissionError(errno.EPERM, 'child denied')
        child.wait.side_effect = subprocess.TimeoutExpired('owned child', .01)
        with mock.patch.object(O.os, 'killpg',
                               side_effect=PermissionError(errno.EPERM, 'group denied')):
            with self.assertRaises(O.ChildCleanupError) as caught:
                O.cleanup_child(child, .01, .02)
        self.assertIsNone(caught.exception.report['returncode'])
        self.assertEqual(child.send_signal.call_args_list,
                         [mock.call(signal.SIGTERM), mock.call(signal.SIGKILL)])
        child.wait.assert_called_once_with(timeout=.02)
        self.assertTrue(caught.exception.cleanup_errors)

    @unittest.skipUnless(os.name == 'posix', 'POSIX process groups required')
    def test_group_permission_denial_still_terminates_owned_stopped_child(self):
        child = subprocess.Popen([sys.executable, '-c', 'import time;time.sleep(30)'],
                                 start_new_session=True)
        try:
            os.killpg(child.pid, signal.SIGSTOP)
            with mock.patch.object(O.os, 'killpg',
                                   side_effect=PermissionError(errno.EPERM, 'group denied')):
                report = O.cleanup_child(child, 2, 2)
            self.assertEqual(report['returncode'], -signal.SIGTERM)
            self.assertEqual([item['signal'] for item in report['signals']],
                             ['SIGTERM', 'SIGCONT'])
            self.assertTrue(all(item['scope'] == 'child' for item in report['signals']))
            self.assertEqual(len(report['errors']), 2)
        finally:
            if child.poll() is None:
                child.kill()
                child.wait(timeout=3)

    def test_cleanup_permission_failure_cannot_hide_primary_journal_error(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            original_save = O.save
            primary = OSError(errno.ENOSPC, 'primary full journal')
            child = mock.Mock(pid=424242, returncode=None)
            child.poll.return_value = None
            child.send_signal.side_effect = PermissionError(errno.EPERM, 'child denied')
            child.wait.side_effect = subprocess.TimeoutExpired('owned child', .01)

            def fail_running(path, document):
                if document['status'] == 'running':
                    raise primary
                original_save(path, document)

            with mock.patch.object(O, 'snapshot', return_value={}), \
                 mock.patch.object(O, 'save', side_effect=fail_running), \
                 mock.patch.object(O.subprocess, 'Popen', return_value=child) as spawn, \
                 mock.patch.object(O.os, 'killpg',
                                   side_effect=PermissionError(errno.EPERM, 'group denied')):
                with self.assertRaises(OSError) as caught:
                    O.observe(['synthetic child'], root, .01, .01)
            self.assertIs(caught.exception, primary)
            spawn.assert_called_once()
            saved = json.loads((root/'observer-status.json').read_text())
            self.assertEqual(saved['error']['error_type'], 'OSError')
            self.assertIn('primary full journal', saved['error']['error'])
            self.assertTrue(saved['error']['cleanup_incomplete'])
            self.assertIsNone(saved['error']['child_returncode'])
            self.assertTrue(any(item['error_type'] == 'PermissionError'
                                for item in saved['error']['cleanup_errors']))
            self.assertEqual(saved['terminal_journal_reserve']['released_bytes'],
                             O.EmergencyReserve.size)
            with (root/'observer.lock').open('a') as lock:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)

    def test_terminal_reserve_allocation_failure_prevents_child_launch(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            allocate = O.EmergencyReserve.allocate

            def allocate_with_full_disk(reserve):
                if reserve.path.name == '.observer-terminal-reserve':
                    raise OSError(errno.ENOSPC, 'cannot reserve terminal journal')
                allocate(reserve)

            with mock.patch.object(O.EmergencyReserve, 'allocate', allocate_with_full_disk):
                with mock.patch.object(O.subprocess, 'Popen') as spawn:
                    with self.assertRaises(OSError):
                        self.run_child('pass', root)
                    spawn.assert_not_called()
            self.assertFalse((root / 'attempt-0001.stdout').exists())
            self.assertFalse((root / '.observer-emergency-reserve').exists())
            self.assertFalse((root / '.observer-terminal-reserve').exists())

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
            self.assertNotIn('child_pid', state)
            retried = self.run_child("print('started')", root)
            self.assertEqual(retried['attempt'], 2)
            self.assertEqual(retried['outcome']['returncode'], 0)

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

    def test_previous_unconfirmed_cleanup_refuses_live_or_unknown_child(self):
        for pid, error in ((None, {'cleanup_incomplete': True, 'child_returncode': None}),
                           (None, {'child_returncode': None}),
                           (424242, {'child_returncode': None}),
                           (424242, {'cleanup_incomplete': True, 'child_returncode': 1})):
            with self.subTest(pid=pid, error=error), tempfile.TemporaryDirectory() as td:
                root = Path(td)
                previous = {'status':'observer_error', 'attempt':1, 'child_pid':pid,
                            'error':error}
                O.save(root/'observer-status.json', previous)
                with mock.patch.object(O, 'alive', return_value=True), \
                     mock.patch.object(O.subprocess, 'Popen') as spawn:
                    with self.assertRaisesRegex(RuntimeError, 'refusing duplicate'):
                        self.run_child('must not launch', root)
                spawn.assert_not_called()
                self.assertEqual(json.loads((root/'observer-status.json').read_text()), previous)
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
