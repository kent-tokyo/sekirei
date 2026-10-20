"""Capacity guard lifecycle tests; all process observations/signals are fake."""
from dataclasses import replace
import json
import os
from pathlib import Path
import signal
import subprocess
import time
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import guard_halfkp_capacity as G


class FakeSystem:
    def __init__(self):
        self.table = {
            100: G.Process(100, 1, 100, '1', 'Wed Oct 21 07:16:22 2026', '/python', 'S'),
            200: G.Process(200, 100, 200, '200', 'Wed Oct 21 07:16:23 2026', '/python', 'S'),
            300: G.Process(300, 200, 300, '300', 'Wed Oct 21 07:16:24 2026', '/engine', 'S'),
            900: G.Process(900, 1, 900, '900', 'Wed Oct 21 07:16:25 2026', '/unrelated', 'S'),
        }
        self.free = 7000
        self.signals = []
        self.on_stop = None
        self.sleep_error = None
        self.before_signal = None

    def processes(self):
        return dict(self.table)

    def free_mib(self, path):
        return self.free

    def signal_group(self, pgid, number):
        if self.before_signal:
            self.before_signal(pgid, number)
        self.signals.append((pgid, number))
        for pid, p in list(self.table.items()):
            if p.pgid == pgid:
                self.table[pid] = replace(p, state='T' if number == signal.SIGSTOP else 'S')
        if number == signal.SIGSTOP and self.on_stop:
            self.on_stop(pgid)

    def sleep(self, seconds):
        if self.sleep_error:
            raise self.sleep_error


class GuardTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name).resolve()
        self.trial = self.root/'trial'; self.trial.mkdir()
        self.script = self.trial/'controller.py'; self.script.write_text('# frozen fake controller\n')
        plan = {'bound_inputs': [{'path': str(self.script), 'sha256': G.sha(self.script)}],
                'generator': '/engine', 'packer': '/pack'}
        G.save(self.trial/'plan.json', plan)
        self.system = FakeSystem()
        controller = self.system.table[200]; observer = self.system.table[100]
        self.argv = ['/python', '-u', '-B', str(self.script), '--plan', str(self.trial/'plan.json')]
        G.save(self.trial/'status.json', {'pid': 200, 'plan': {
            'path': str(self.trial/'plan.json'), 'sha256': G.sha(self.trial/'plan.json')}})
        self.obs = self.trial/'observer-status.json'
        G.save(self.obs, {'status': 'running', 'observer_pid': 100, 'child_pid': 200,
                          'command': self.argv})
        self.config = self.root/'config.json'
        G.save(self.config, {'schema': G.SCHEMA, 'trial_dir': str(self.trial),
                            'state_dir': str(self.root/'guard'), 'guard_sha256': G.sha(G.__file__),
                            'plan_sha256': G.sha(self.trial/'plan.json'),
                            'controller_script': str(self.script),
                            'controller_script_sha256': G.sha(self.script),
                            'controller': controller.identity(), 'observer': observer.identity(),
                            'controller_argv': self.argv, 'observer_status': str(self.obs),
                            'pause_below_mib': 2048, 'resume_at_mib': 3072,
                            'stable_samples': 3, 'poll_seconds': 2})
        self.guard = G.Guard(self.config, self.system)

    def tearDown(self):
        self.guard.close()
        self.tmp.cleanup()

    def test_ps_parser_handles_single_digit_day_and_path_spaces(self):
        table = G.parse_ps('200 100 200 0 Wed Oct  7 07:16:23 2026 TNs /some path/Python\n')
        self.assertEqual(table[200].started, 'Wed Oct 7 07:16:23 2026')
        self.assertEqual(table[200].command, '/some path/Python')
        self.assertEqual(table[200].state, 'TNs')

    def test_low_disk_freezes_controller_first_then_child_with_durable_intent(self):
        journal = self.root/'guard/guard-status.json'
        def check(pgid, number):
            doc = json.loads(journal.read_text())
            self.assertIn(pgid, [x['leader']['pid'] for x in doc['groups']])
        self.system.before_signal = check
        self.system.free = 2047
        self.guard.tick()
        self.assertEqual(self.system.signals, [(200, signal.SIGSTOP), (300, signal.SIGSTOP)])
        self.assertEqual(self.guard.state['status'], 'PAUSED')
        self.assertEqual(self.system.table[100].state, 'S')
        self.assertEqual(self.system.table[900].state, 'S')

    def test_resume_requires_three_capacity_samples_and_resets_on_pressure(self):
        self.system.free = 1000; self.guard.tick()
        for free in (3072, 3072, 3071, 3072, 3072):
            self.system.free = free; self.guard.tick()
        self.assertFalse(any(s == signal.SIGCONT for _, s in self.system.signals))
        self.system.free = 3072; self.guard.tick()
        self.assertEqual(self.system.signals[-2:], [(300, signal.SIGCONT), (200, signal.SIGCONT)])
        self.assertEqual(self.guard.state['groups'], [])

    def test_freezes_child_spawned_before_controller_stop_was_confirmed(self):
        self.system.table.pop(300)
        def spawn(pgid):
            if pgid == 200:
                self.system.table[301] = G.Process(301, 200, 301, '301', 'new', '/engine', 'S')
        self.system.on_stop = spawn
        self.system.free = 1000; self.guard.tick()
        self.assertEqual(self.system.signals, [(200, signal.SIGSTOP), (301, signal.SIGSTOP)])

    def test_reused_controller_pid_or_changed_observer_gets_no_signal(self):
        self.system.table[200] = replace(self.system.table[200], started='different incarnation')
        self.system.free = 1000
        self.assertFalse(self.guard.tick())
        self.assertEqual(self.system.signals, [])

    def test_wrong_observer_identity_is_rejected_before_pause(self):
        self.system.table[100] = replace(self.system.table[100], started='wrong observer')
        self.system.free = 1000
        with self.assertRaisesRegex(ValueError, 'observer incarnation'):
            self.guard.tick()
        self.assertEqual(self.system.signals, [])

    def test_unrelated_group_member_is_never_signalled(self):
        self.system.table[900] = replace(self.system.table[900], pgid=200, session='200')
        self.system.free = 1000
        with self.assertRaisesRegex(ValueError, 'unverified'):
            self.guard.tick()
        self.assertEqual(self.system.signals, [])

    def test_reused_paused_child_pid_is_not_continued(self):
        self.system.free = 1000; self.guard.tick()
        self.system.table[300] = replace(self.system.table[300], started='new', ppid=1)
        self.guard.resume('operator_shutdown')
        self.assertEqual(self.system.signals[-1], (200, signal.SIGCONT))
        self.assertNotIn((300, signal.SIGCONT), self.system.signals)

    def test_caught_shutdown_continues_owned_groups_without_restarting_child(self):
        self.system.free = 1000
        self.system.sleep_error = InterruptedError('operator shutdown')
        with self.assertRaises(InterruptedError):
            self.guard.run()
        self.assertEqual(self.system.signals, [(200, signal.SIGSTOP), (300, signal.SIGSTOP),
                                              (300, signal.SIGCONT), (200, signal.SIGCONT)])
        self.assertEqual(json.loads(self.guard.state_path.read_text())['status'], 'STOPPED')

    def test_entry_log_failure_still_cleans_up_previously_paused_groups(self):
        self.system.free = 1000; self.guard.tick()
        event = self.guard.event
        calls = 0
        def fail_once(name, **values):
            nonlocal calls
            calls += 1
            if calls == 1:
                raise OSError('injected initial event failure')
            return event(name, **values)
        with patch.object(self.guard, 'event', side_effect=fail_once):
            with self.assertRaisesRegex(OSError, 'initial event failure'):
                self.guard.run()
        self.assertEqual(self.system.signals[-2:], [(300, signal.SIGCONT), (200, signal.SIGCONT)])
        self.assertEqual(self.guard.state['groups'], [])

    def test_second_guard_lock_fails_and_same_journal_can_reconcile_guard_death(self):
        with self.assertRaises(BlockingIOError):
            G.Guard(self.config, self.system)
        self.system.free = 1000; self.guard.tick()
        self.guard.close()  # simulate disappeared guard, preserving paused groups/journal
        self.guard = G.Guard(self.config, self.system)
        for _ in range(3):
            self.system.free = 3072; self.guard.tick()
        self.assertEqual(self.system.signals[-2:], [(300, signal.SIGCONT), (200, signal.SIGCONT)])
        self.assertEqual(len([x for x in self.system.signals if x[1] == signal.SIGSTOP]), 2)

    def test_frozen_config_or_plan_mutation_fails_before_signalling(self):
        with (self.trial/'plan.json').open('a') as stream:
            stream.write(' ')
        self.system.free = 1000
        with self.assertRaisesRegex(ValueError, 'plan changed'):
            self.guard.tick()
        self.assertEqual(self.system.signals, [])

    def test_unknown_observations_preserve_paused_groups_and_reset_recovery(self):
        self.system.free = 1000; self.guard.tick()
        self.system.free = 3072; self.guard.tick()
        with patch.object(self.system, 'processes', side_effect=G.ObservationUnavailable('timeout')):
            self.guard.tick()
        self.assertEqual(self.guard.state['status'], 'PAUSED')
        self.assertEqual(self.guard.state['stable_samples'], 0)
        self.assertEqual(len(self.guard.state['groups']), 2)
        with patch.object(self.system, 'free_mib', side_effect=G.ObservationUnavailable('disk stat')):
            self.guard.tick()
        self.assertFalse(any(s == signal.SIGCONT for _, s in self.system.signals))
        self.guard.tick(); self.guard.tick()
        self.assertFalse(any(s == signal.SIGCONT for _, s in self.system.signals))
        self.guard.tick()
        self.assertEqual(self.system.signals[-2:], [(300, signal.SIGCONT), (200, signal.SIGCONT)])

    def test_system_wraps_timeout_malformed_processes_and_capacity_errors(self):
        for error in (subprocess.TimeoutExpired('ps', 10), OSError('not available')):
            with patch.object(G.subprocess, 'run', side_effect=error):
                with self.assertRaises(G.ObservationUnavailable):
                    G.System().processes()
        with patch.object(G.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, 'bad row')):
            with self.assertRaises(G.ObservationUnavailable):
                G.System().processes()
        with patch.object(G.shutil, 'disk_usage', side_effect=OSError('not available')):
            with self.assertRaises(G.ObservationUnavailable):
                G.System().free_mib(self.trial)

    def test_operator_shutdown_is_not_swallowed_as_transient_observation(self):
        with patch.object(G.subprocess, 'run', side_effect=G.GuardShutdown('stop')):
            with self.assertRaises(G.GuardShutdown):
                G.System().processes()
        with patch.object(G.shutil, 'disk_usage', side_effect=G.GuardShutdown('stop')):
            with self.assertRaises(G.GuardShutdown):
                G.System().free_mib(self.trial)
        with self.assertRaises(G.GuardShutdown):
            G.interrupted(signal.SIGTERM, None)

    def test_all_pilot_inputs_revalidated_before_capacity_cont(self):
        payload = self.trial/'input.bin'; payload.write_bytes(b'original')
        plan = json.loads((self.trial/'plan.json').read_text())
        plan['bound_inputs'].append({'path': str(payload), 'sha256': G.sha(payload)})
        G.save(self.trial/'plan.json', plan)
        # Freeze the updated synthetic plan before starting this synthetic guard.
        self.guard.close()
        config = json.loads(self.config.read_text())
        config['plan_sha256'] = G.sha(self.trial/'plan.json')
        G.save(self.config, config)
        status = json.loads((self.trial/'status.json').read_text())
        status['plan']['sha256'] = config['plan_sha256']; G.save(self.trial/'status.json', status)
        self.guard = G.Guard(self.config, self.system)
        self.system.free = 1000; self.guard.tick()
        payload.write_bytes(b'modified')
        self.system.free = 3072
        self.guard.tick(); self.guard.tick()
        with self.assertRaisesRegex(ValueError, 'input changed before resume'):
            self.guard.tick()
        self.assertFalse(any(s == signal.SIGCONT for _, s in self.system.signals))

    def test_interrupted_resume_is_refrozen_before_new_recovery_samples(self):
        self.system.free = 1000; self.guard.tick()
        # Simulate guard disappearance just after child CONT; controller still T.
        self.system.table[300] = replace(self.system.table[300], state='S')
        self.guard.state['groups'] = [x for x in self.guard.state['groups'] if x['role'] == 'controller']
        self.guard.state['status'] = 'RESUMING'; self.guard.persist(); self.guard.close()
        self.guard = G.Guard(self.config, self.system)
        self.system.free = 3072; self.guard.tick()
        self.assertEqual(self.system.table[300].state, 'T')
        self.assertEqual(self.system.signals[-1], (300, signal.SIGSTOP))
        self.assertEqual(self.guard.state['stable_samples'], 1)

    def test_observer_completion_race_is_target_exit_when_root_confirmed_gone(self):
        stale = self.system.processes()
        G.save(self.obs, {'status': 'exited'})
        with patch.object(self.system, 'processes', side_effect=[stale, {100: stale[100]}]):
            self.assertFalse(self.guard.tick())
        self.assertEqual(self.guard.state['status'], 'TARGET_EXITED')
        self.assertEqual(self.system.signals, [])

    def test_unknown_preexisting_pause_is_not_adopted(self):
        self.system.table[200] = replace(self.system.table[200], state='T')
        self.system.free = 1000
        with self.assertRaisesRegex(ValueError, 'before this guard owned'):
            self.guard.tick()
        self.assertEqual(self.system.signals, [])


@unittest.skipUnless(sys.platform == 'darwin' and hasattr(os, 'getsid'), 'macOS process-session integration')
class RealProcessIntegrationTest(unittest.TestCase):
    def test_owned_synthetic_controller_and_child_stop_and_continue(self):
        class Capacity(G.System):
            free = 7000
            def free_mib(self, path):
                return self.free
        system = Capacity()
        with tempfile.TemporaryDirectory() as td:
            root = Path(td).resolve(); trial = root/'trial'; trial.mkdir()
            script = trial/'controller.py'
            script.write_text("""import subprocess,sys,time,signal
from pathlib import Path
child=subprocess.Popen([sys.executable,'-c','import time;time.sleep(60)'],start_new_session=True)
Path(__file__).with_suffix('.child').write_text(str(child.pid))
def stop(number,frame): raise SystemExit(0)
signal.signal(signal.SIGTERM,stop)
try:
 while True: time.sleep(.05)
finally:
 child.terminate();child.wait(timeout=5)
""")
            plan = {'generator': sys.executable, 'packer': sys.executable,
                    'bound_inputs': [{'path': str(script), 'sha256': G.sha(script)}]}
            G.save(trial/'plan.json', plan)
            argv = [sys.executable, '-u', '-B', str(script), '--plan', str(trial/'plan.json')]
            controller = subprocess.Popen(argv, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                          stderr=subprocess.DEVNULL, start_new_session=True)
            guard = None
            try:
                deadline = time.monotonic()+5
                while not script.with_suffix('.child').exists():
                    self.assertLess(time.monotonic(), deadline)
                    time.sleep(.01)
                child = int(script.with_suffix('.child').read_text())
                table = system.processes(); owner = table[controller.pid]; observer = table[os.getpid()]
                G.save(trial/'status.json', {'pid': controller.pid,
                        'plan': {'path': str(trial/'plan.json'), 'sha256': G.sha(trial/'plan.json')}})
                obs = root/'observer.json'
                G.save(obs, {'status': 'running', 'observer_pid': os.getpid(),
                            'child_pid': controller.pid, 'command': argv})
                config = root/'config.json'
                G.save(config, {'schema': G.SCHEMA, 'guard_sha256': G.sha(G.__file__),
                        'trial_dir': str(trial), 'state_dir': str(root/'guard'),
                        'plan_sha256': G.sha(trial/'plan.json'),
                        'controller_script': str(script), 'controller_script_sha256': G.sha(script),
                        'controller': owner.identity(), 'observer': observer.identity(),
                        'controller_argv': argv, 'observer_status': str(obs),
                        'pause_below_mib': 2048, 'resume_at_mib': 3072,
                        'stable_samples': 3, 'poll_seconds': .05})
                guard = G.Guard(config, system)
                system.free = 1000; guard.tick()
                table = system.processes()
                self.assertIn('T', table[controller.pid].state)
                self.assertIn('T', table[child].state)
                self.assertNotIn('T', table[os.getpid()].state)
                system.free = 3072
                for _ in range(3): guard.tick()
                table = system.processes()
                self.assertNotIn('T', table[controller.pid].state)
                self.assertNotIn('T', table[child].state)
                self.assertEqual(guard.state['groups'], [])
            finally:
                if guard is not None:
                    try:
                        guard.resume('integration_cleanup')
                    finally:
                        guard.close()
                if controller.poll() is None:
                    controller.terminate()
                    controller.wait(timeout=5)


if __name__ == '__main__':
    unittest.main()
