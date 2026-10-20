#!/usr/bin/env python3
"""Pause one existing, identity-pinned HalfKP pilot during disk pressure.

This is capacity supervision, never a process/job retry. Only the controller
and its verified descendant groups receive STOP/CONT; its observer stays live.
Incarnations use PID/start-time/session/command and are checked immediately
before each signal. macOS has no portable atomic incarnation-bound signal API,
so a very small check/signal race cannot be eliminated. Guard SIGKILL or OS
failure may leave owned processes stopped; the same frozen config and journal
can be inspected/reconciled by a later guard. Caught shutdown resumes owned
paused groups. Old controller wall time continues to include the pause.
"""
from __future__ import annotations
import argparse
from dataclasses import asdict, dataclass, replace
from datetime import datetime, timezone
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import time

SCHEMA = 'sekirei.halfkp-capacity-guard.v1'


class GuardShutdown(RuntimeError):
    """A caught operator signal; never classify this as missing telemetry."""


class ObservationUnavailable(RuntimeError):
    """An observation failed; never authorize resume from missing telemetry."""


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(path):
    digest = hashlib.sha256()
    with Path(path).open('rb') as stream:
        for block in iter(lambda: stream.read(1048576), b''):
            digest.update(block)
    return digest.hexdigest()


def now():
    return datetime.now(timezone.utc).isoformat()


def save(path, value):
    pending = path.with_name('.' + path.name + '.pending')
    with pending.open('w') as stream:
        json.dump(value, stream, indent=2, allow_nan=False)
        stream.write('\n'); stream.flush(); os.fsync(stream.fileno())
    os.replace(pending, path)


@dataclass(frozen=True)
class Process:
    pid: int
    ppid: int
    pgid: int
    session: str
    started: str
    command: str
    state: str = ''

    def identity(self):
        return {k: v for k, v in asdict(self).items() if k != 'state'}

    def matches(self, identity, parent=True):
        keys = ('pid', 'pgid', 'session', 'started', 'command')
        return all(getattr(self, k) == identity[k] for k in keys) and (
            not parent or self.ppid == identity['ppid'])


def parse_ps(text):
    result = {}
    for line in text.splitlines():
        if not line.strip():
            continue
        fields = line.split(None, 10)
        require(len(fields) == 11, 'malformed process observation')
        process = Process(int(fields[0]), int(fields[1]), int(fields[2]), fields[3],
                          ' '.join(fields[4:9]), fields[10], fields[9])
        require(process.pid not in result, 'duplicate observed PID')
        result[process.pid] = process
    return result


class System:
    def processes(self):
        try:
            result = subprocess.run(['/bin/ps', '-axo',
                                     'pid=,ppid=,pgid=,sess=,lstart=,stat=,comm='],
                                    capture_output=True, text=True, check=True, timeout=10)
            table = parse_ps(result.stdout)
            # macOS may redact ps sess to zero; getsid supplies the real SID.
            observed = {}
            for pid, process in table.items():
                try:
                    observed[pid] = replace(process, session=str(os.getsid(pid)))
                except ProcessLookupError:
                    continue
                except PermissionError:
                    observed[pid] = replace(process, session="unavailable")
            require(observed, 'empty process observation')
            return observed
        except (OSError, ValueError, subprocess.SubprocessError) as error:
            raise ObservationUnavailable('process observation: ' + type(error).__name__) from error

    def free_mib(self, path):
        try:
            return shutil.disk_usage(path).free // 1048576
        except OSError as error:
            raise ObservationUnavailable('capacity observation: ' + type(error).__name__) from error

    def signal_group(self, pgid, number):
        os.killpg(pgid, number)

    def sleep(self, seconds):
        time.sleep(seconds)


def descendant(pid, ancestor, table):
    visited = set()
    while pid in table and pid not in visited:
        if pid == ancestor:
            return True
        visited.add(pid)
        pid = table[pid].ppid
    return False


class Guard:
    def __init__(self, config_path, system=None):
        self.path = Path(config_path).resolve()
        self.config_hash = sha(self.path)
        self.config = json.loads(self.path.read_text())
        self.system = system or System()
        c = self.config
        require(c.get('schema') == SCHEMA, 'unknown guard config schema')
        require(c['guard_sha256'] == sha(__file__), 'guard source changed')
        require(c['pause_below_mib'] == 2048 and c['resume_at_mib'] == 3072 and
                c['stable_samples'] == 3, 'unexpected capacity policy')
        require(type(c['poll_seconds']) in (int, float) and 0 < c['poll_seconds'] <= 5,
                'poll interval must be positive and at most five seconds')
        self.trial = Path(c['trial_dir']).resolve()
        self.directory = Path(c['state_dir']).resolve()
        require(self.directory != self.trial and self.trial not in self.directory.parents,
                'guard state must be outside the frozen trial')
        self.directory.mkdir(parents=True, exist_ok=True)
        self.lock = (self.directory / 'guard.lock').open('a')
        try:
            fcntl.flock(self.lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BaseException:
            self.lock.close()
            raise
        self.state_path = self.directory / 'guard-status.json'
        self.event_path = self.directory / 'guard-events.jsonl'
        self.state = {'schema': SCHEMA, 'config_sha256': self.config_hash,
                      'status': 'WATCHING', 'groups': [], 'stable_samples': 0,
                      'started_at': now(), 'guard_pid': os.getpid(), 'automatic_retry': False}
        if self.state_path.exists():
            old = json.loads(self.state_path.read_text())
            require(old['config_sha256'] == self.config_hash, 'journal/config mismatch')
            require(old.get('status') not in ('TARGET_EXITED', 'STOPPED'),
                    'closed guard cannot automatically reopen')
            self.state.update(old, guard_pid=os.getpid(), stable_samples=0)
        self.verify_files()

    def close(self):
        self.lock.close()

    def persist(self):
        self.state['updated_at'] = now()
        save(self.state_path, self.state)

    def event(self, name, **values):
        with self.event_path.open('a') as stream:
            stream.write(json.dumps({'at': now(), 'event': name, **values}, allow_nan=False) + '\n')
            stream.flush(); os.fsync(stream.fileno())

    def verify_files(self):
        c = self.config
        require(sha(self.path) == self.config_hash, 'guard config changed')
        require(sha(self.trial / 'plan.json') == c['plan_sha256'], 'pilot plan changed')
        require(sha(c['controller_script']) == c['controller_script_sha256'],
                'frozen controller script changed')
        require(sha(__file__) == c['guard_sha256'], 'guard source changed')
        plan = json.loads((self.trial / 'plan.json').read_text())
        bound = {str(Path(x['path']).resolve()): x['sha256'] for x in plan['bound_inputs']}
        require(bound.get(str(Path(c['controller_script']).resolve())) == c['controller_script_sha256'],
                'controller is not a frozen plan input')

    def watch_identity(self, table):
        c = self.config
        root = table.get(c['controller']['pid'])
        if root is None or not root.matches(c['controller'], parent=False):
            return False
        observer = table.get(c['observer']['pid'])
        if observer is None or not observer.matches(c['observer']):
            fresh = self.system.processes().get(c['controller']['pid'])
            if fresh is None or not fresh.matches(c['controller'], parent=False):
                return False
            raise ValueError('observer incarnation changed')
        require(root.ppid == observer.pid and root.pgid == root.pid and
                root.pgid != observer.pgid and root.session == str(root.pid) and
                observer.session != 'unavailable' and root.session != observer.session,
                'controller/observer ownership or session mismatch')
        status = json.loads((self.trial / 'status.json').read_text())
        require(status['pid'] == root.pid and status['plan']['sha256'] == c['plan_sha256'] and
                Path(status['plan']['path']).resolve() == self.trial / 'plan.json',
                'controller status identity mismatch')
        observer_status = json.loads(Path(c['observer_status']).read_text())
        if observer_status['status'] != 'running':
            fresh = self.system.processes().get(c['controller']['pid'])
            if fresh is None or not fresh.matches(c['controller'], parent=False):
                return False
        require(observer_status['status'] == 'running' and
                observer_status['observer_pid'] == observer.pid and
                observer_status['child_pid'] == root.pid and
                observer_status['command'] == c['controller_argv'], 'observer status identity mismatch')
        require(str(Path(c['controller_script']).resolve()) in c['controller_argv'] and
                '--plan' in c['controller_argv'] and
                c['controller_argv'][c['controller_argv'].index('--plan') + 1] == str(self.trial / 'plan.json'),
                'unexpected observed controller command')
        return True

    def verify_group(self, identity, table, parent=True):
        leader = table.get(identity['pid'])
        if leader is None or not leader.matches(identity, parent=parent):
            return None
        require(leader.pgid == leader.pid, 'owned leader is not its group leader')
        members = [p for p in table.values() if p.pgid == leader.pgid]
        require(all(p.session == leader.session and descendant(p.pid, leader.pid, table)
                    for p in members), 'group contains unverified processes')
        return leader

    def confirm_state(self, identity, stopped):
        for _ in range(20):
            table = self.system.processes()
            leader = self.verify_group(identity, table, parent=False)
            if leader is None:
                return False
            if ('T' in leader.state) == stopped:
                return True
            self.system.sleep(.05)
        raise ValueError('signal did not reach confirmed process state')

    def stop_group(self, process, role):
        identity = process.identity()
        existing = next((x for x in self.state['groups'] if x['leader'] == identity), None)
        if existing is None:
            require('T' not in process.state, 'process was stopped before this guard owned it')
            self.state['groups'].append({'leader': identity, 'role': role})
            self.state['status'] = 'PAUSING'
            self.persist()  # Intent must survive a guard death before SIGSTOP.
        table = self.system.processes()
        leader = self.verify_group(identity, table)
        if leader is None:
            self.state['groups'] = [x for x in self.state['groups'] if x['leader'] != identity]
            self.persist()
            return
        if 'T' not in leader.state:
            self.system.signal_group(leader.pgid, signal.SIGSTOP)
        self.confirm_state(identity, True)
        self.event('group_stopped', role=role, leader=identity)

    def pause(self):
        table = self.system.processes()
        require(self.watch_identity(table), 'target exited before pause')
        root = table[self.config['controller']['pid']]
        self.stop_group(root, 'controller')
        # The controller cannot now spawn a new command or run its old disk rule.
        table = self.system.processes()
        require(self.watch_identity(table), 'target exited during pause')
        root = table[root.pid]
        plan = json.loads((self.trial / 'plan.json').read_text())
        allowed = {str(Path(x).resolve()) for x in (plan['generator'], plan['packer'], root.command)}
        groups = {p.pgid for p in table.values() if descendant(p.pid, root.pid, table)} - {root.pgid}
        for pgid in sorted(groups):
            leader = table.get(pgid)
            require(leader is not None and descendant(pgid, root.pid, table) and
                    leader.session == str(leader.pid) and leader.session != root.session and
                    str(Path(leader.command).resolve()) in allowed,
                    'unknown descendant process group')
            self.stop_group(leader, 'child')
        self.state.update(status='PAUSED', stable_samples=0)
        self.persist()
        self.event('paused')

    def verify_resume_inputs(self):
        self.verify_files()
        plan = json.loads((self.trial / 'plan.json').read_text())
        for item in plan['bound_inputs']:
            require(sha(item['path']) == item['sha256'], 'pilot input changed before resume')

    def resume(self, reason):
        if reason == 'capacity_recovered':
            self.verify_resume_inputs()
            if self.system.free_mib(self.trial) < 3072:
                self.state['stable_samples'] = 0
                self.persist()
                return
        # A crash halfway through CONT must be reconciled by freezing again.
        self.state['status'] = 'RESUMING'
        self.persist()
        # Child first, controller last. Only previously journaled incarnations.
        pending = sorted(self.state['groups'], key=lambda x: x['role'] == 'controller')
        for item in pending:
            identity = item['leader']
            table = self.system.processes()
            leader = self.verify_group(identity, table, parent=False)
            if leader is not None:
                self.system.signal_group(leader.pgid, signal.SIGCONT)
                self.confirm_state(identity, False)
                self.event('group_continued', reason=reason, leader=identity)
            else:
                # Never signal a group merely because its numeric PID still exists.
                self.event('incarnation_gone', reason=reason, leader=identity)
            self.state['groups'] = [x for x in self.state['groups'] if x != item]
            self.persist()
        self.state.update(status='WATCHING', stable_samples=0)
        self.persist()

    def tick(self):
        try:
            return self.observed_tick()
        except ObservationUnavailable as error:
            # Keep the existing pause phase and all write-ahead group identities.
            # Unknown telemetry cannot count toward the three recovery samples.
            self.state.update(observation='UNKNOWN', observation_error=str(error), stable_samples=0)
            if not self.state['groups']:
                self.state['status'] = 'WAITING_OBSERVATION'
            self.persist()
            self.event('observation_unavailable', error=str(error))
            return True

    def observed_tick(self):
        self.verify_files()
        table = self.system.processes()
        if not self.watch_identity(table):
            self.resume('target_exit')
            self.state['status'] = 'TARGET_EXITED'; self.persist()
            return False
        free = self.system.free_mib(self.trial)
        self.state.update(free_mib=free, observation='AVAILABLE')
        if self.state['status'] in ('PAUSING', 'PAUSED', 'RESUMING') or self.state['groups']:
            # Reconcile an unfinished pause after guard death using the same journal.
            if self.state['status'] in ('PAUSING', 'RESUMING'):
                self.pause()
            for item in self.state['groups']:
                leader = self.verify_group(item['leader'], self.system.processes(), parent=False)
                require(leader is None or 'T' in leader.state, 'owned group unexpectedly resumed')
            self.state['stable_samples'] = self.state['stable_samples'] + 1 if free >= 3072 else 0
            self.persist()
            if self.state['stable_samples'] >= 3:
                self.resume('capacity_recovered')
        elif free < 2048:
            self.pause()
        else:
            self.state['status'] = 'WATCHING'
            self.persist()
        return True

    def run(self):
        try:
            self.event('guard_started', config_sha256=self.config_hash)
            while self.tick():
                self.system.sleep(self.config['poll_seconds'])
        except BaseException as error:
            self.state.update(status='ERROR', error_type=type(error).__name__, error=str(error))
            try:
                self.resume('caught_guard_shutdown')
            finally:
                self.state.update(status='STOPPED' if not self.state['groups'] else 'NEEDS_OPERATOR')
                self.persist()
            raise
        finally:
            self.close()


def interrupted(number, _frame):
    raise GuardShutdown('guard received signal ' + str(number))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', type=Path, required=True)
    args = parser.parse_args()
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    Guard(args.config).run()


if __name__ == '__main__':
    main()
