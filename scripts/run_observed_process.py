#!/usr/bin/env python3
"""Run one child and durably record its observed exit; never retry it.

A parent can observe a killed child's signal even when that child cannot write
logs. A signal such as SIGKILL does not establish an OOM cause. If this observer
itself dies, a later invocation records an unclosed attempt as unknown and
refuses to duplicate a still-live direct child. This tool observes the direct
child leader, not the lifetime of descendants that detach into other process
groups; a structured runner must own those descendants. Use an OS supervisor for this observer
when independence from an interactive app/session is required.
"""
from __future__ import annotations

import argparse
import errno
from datetime import datetime, timezone
import fcntl
import json
import math
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import time


def now() -> str:
    return datetime.now(timezone.utc).isoformat()


def save(path: Path, document: dict) -> None:
    pending = path.with_name("." + path.name + ".pending")
    with pending.open("w", encoding="utf-8") as stream:
        json.dump(document, stream, indent=2, allow_nan=False)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(pending, path)


def append(path: Path, document: dict) -> None:
    # A failed append can leave half a JSON line. Roll it back before a retry.
    payload = (json.dumps(document, allow_nan=False) + "\n").encode("utf-8")
    with path.open("ab", buffering=0) as stream:
        offset = stream.tell()
        try:
            remaining = memoryview(payload)
            while remaining:
                written = stream.write(remaining)
                if not written:
                    raise OSError(errno.ENOSPC, "event append made no progress")
                remaining = remaining[written:]
            os.fsync(stream.fileno())
        except OSError:
            os.ftruncate(stream.fileno(), offset)
            raise


class EmergencyReserve:
    """Physically reserve a bounded exit-journal budget; consume it only once.

    Do not replenish during an attempt: further disk exhaustion fails closed
    instead of repeatedly writing or relaunching the child.
    """

    size = 8 * 1048576

    def __init__(self, directory: Path):
        self.path = directory / ".observer-emergency-reserve"
        self.available = False
        self.recovery = None

    def allocate(self) -> None:
        # The caller holds the observer lock and has checked previous identity.
        self.path.unlink(missing_ok=True)
        try:
            with self.path.open("xb") as stream:
                block = bytes(65536)
                for _ in range(self.size // len(block)):
                    stream.write(block)
                stream.flush()
                os.fsync(stream.fileno())
            self.available = True
        except BaseException:
            self.close()
            raise

    def write(self, operation, path: Path, document: dict) -> None:
        try:
            operation(path, document)
        except OSError as error:
            if error.errno != errno.ENOSPC or not self.available:
                raise
            self.close()
            self.recovery = {"at": now(), "reason": "ENOSPC",
                             "released_bytes": self.size,
                             "automatic_child_retry": False}
            # One retry of journal persistence only, never of the child.
            document["persistence_recovery"] = self.recovery
            operation(path, document)

    def close(self) -> None:
        self.path.unlink(missing_ok=True)
        self.available = False


def describe_exit(returncode: int, requested_signal: int | None = None) -> dict:
    result = {"returncode": returncode, "stop_requested_signal": requested_signal,
              "kind": "normal_exit" if returncode == 0 else "nonzero_exit"}
    if returncode < 0:
        number = -returncode
        try:
            name = signal.Signals(number).name
        except ValueError:
            name = "UNKNOWN"
        result.update(kind="signal", signal_number=number, signal_name=name,
                      underlying_cause="unknown; signal does not prove OOM or who sent it")
    return result


def snapshot(directory: Path, child_pid: int | None) -> dict:
    result = {"at": now()}
    try:
        result["free_mib"] = shutil.disk_usage(directory).free // 1048576
    except OSError as error:
        result["capacity_error"] = type(error).__name__
    try:
        process = subprocess.run(["ps", "-Ao", "pcpu="], capture_output=True,
                                 text=True, check=True, timeout=5)
        values = [float(line.strip()) for line in process.stdout.splitlines() if line.strip()]
        if not values or not all(math.isfinite(x) and x >= 0 for x in values):
            raise ValueError("invalid host CPU observation")
        result["host_total_cpu_percent"] = sum(values)
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        result["host_cpu_observation_error"] = type(error).__name__
    if child_pid is not None:
        try:
            process = subprocess.run(["ps", "-p", str(child_pid), "-o", "rss=,pcpu=,stat="],
                                     capture_output=True, text=True, timeout=5)
            result["child_resource_observation"] = process.stdout.strip()
        except (OSError, subprocess.SubprocessError) as error:
            result["process_observation_error"] = type(error).__name__
    return result


def alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True


def observe(command: list[str], directory: Path, poll_seconds: float = 10,
            stop_grace_seconds: float = 15) -> dict:
    if (not command or not math.isfinite(poll_seconds) or not math.isfinite(stop_grace_seconds)
            or poll_seconds <= 0 or stop_grace_seconds <= 0):
        raise ValueError("command and positive polling/grace are required")
    directory.mkdir(parents=True, exist_ok=True)
    lock = (directory / "observer.lock").open("a")
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        lock.close()
        raise RuntimeError("an observer already owns this state directory") from None
    state_path = directory / "observer-status.json"
    events = directory / "observer-events.jsonl"
    requested = None
    previous_handlers = {}
    child = None
    state = None
    reserve = EmergencyReserve(directory)

    def persist_state() -> None:
        if reserve.recovery is not None:
            state["persistence_recovery"] = reserve.recovery
        reserve.write(save, state_path, state)
        if reserve.recovery is not None and "persistence_recovery" not in state:
            state["persistence_recovery"] = reserve.recovery
            reserve.write(save, state_path, state)

    def event(name: str, **fields) -> None:
        reserve.write(append, events, {"at": now(), "event": name, **fields})

    def request_stop(number, _frame) -> None:
        nonlocal requested
        requested = requested or number

    try:
        previous = json.loads(state_path.read_text()) if state_path.exists() else {}
        if previous.get("status") == "starting":
            event("previous_spawn_identity_unknown", previous=previous,
                  cause="observer disappeared before child identity was durable")
            raise RuntimeError("previous spawn may have succeeded; resolve unknown child before relaunch")
        if previous.get("status") == "running":
            event("previous_observer_disappeared", previous=previous,
                  cause="unknown; previous observer did not persist its exit")
            pid = previous.get("child_pid")
            if type(pid) is not int or pid < 1:
                raise RuntimeError("previous child identity is unknown; refusing duplicate launch")
            if alive(pid):
                raise RuntimeError("previous child may still be alive; refusing duplicate launch")
        reserve.allocate()
        attempt = int(previous.get("attempt", 0)) + 1
        stdout_path = directory / ("attempt-%04d.stdout" % attempt)
        stderr_path = directory / ("attempt-%04d.stderr" % attempt)
        state = {"schema": "sekirei.observed-process.v1", "status": "starting",
                 "attempt": attempt, "started_at": now(), "observer_pid": os.getpid(),
                 "command": command, "stdout": str(stdout_path), "stderr": str(stderr_path),
                 "automatic_retry": False, "initial_resources": snapshot(directory, None)}
        persist_state()
        event("attempt_starting", attempt=attempt, command=command)
        for number in (signal.SIGTERM, signal.SIGINT):
            previous_handlers[number] = signal.signal(number, request_stop)
        with stdout_path.open("xb") as stdout, stderr_path.open("xb") as stderr:
            child = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=stdout,
                                     stderr=stderr, start_new_session=True)
            state.update(status="running", child_pid=child.pid)
            persist_state()
            event("child_started", attempt=attempt, child_pid=child.pid)
            stop_deadline = None
            while child.poll() is None:
                if requested is not None and stop_deadline is None:
                    event("stop_requested", requested_signal=requested, child_pid=child.pid)
                    try:
                        os.killpg(child.pid, signal.SIGTERM)
                    except ProcessLookupError:
                        pass
                    stop_deadline = time.monotonic() + stop_grace_seconds
                if stop_deadline is not None and time.monotonic() >= stop_deadline:
                    event("stop_grace_expired", child_pid=child.pid)
                    try:
                        os.killpg(child.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    stop_deadline = float("inf")
                try:
                    child.wait(timeout=min(poll_seconds, 1) if requested else poll_seconds)
                except subprocess.TimeoutExpired:
                    pass
                state.update(last_heartbeat=snapshot(directory, child.pid))
                persist_state()
            stdout.flush()
            stderr.flush()
            os.fsync(stdout.fileno())
            os.fsync(stderr.fileno())
        outcome = describe_exit(child.returncode, requested)
        state.update(status="exited", completed_at=now(), outcome=outcome,
                     final_resources=snapshot(directory, child.pid))
        persist_state()
        event("child_exited", attempt=attempt, child_pid=child.pid, outcome=outcome,
              last_heartbeat=state.get("last_heartbeat"), final_resources=state["final_resources"])
        if reserve.recovery is not None and "persistence_recovery" not in state:
            persist_state()
        return state
    except BaseException as error:
        # A persistence failure must not leave an unobserved owned child running.
        if child is not None and child.poll() is None:
            try:
                os.killpg(child.pid, signal.SIGTERM)
                child.wait(timeout=stop_grace_seconds)
            except subprocess.TimeoutExpired:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait()
            except ProcessLookupError:
                pass
        failure = {"at": now(), "event": "observer_error", "error_type": type(error).__name__,
                   "error": str(error), "child_returncode": None if child is None else child.returncode}
        for write_failure in (
                lambda: event("observer_error", **{k: v for k, v in failure.items()
                                                   if k not in {"at", "event"}}),
                lambda: persist_state() if state is not None else None):
            try:
                if state is not None:
                    state.update(status="observer_error", completed_at=now(), error=failure)
                write_failure()
            except OSError:
                # Keep the original error and independently try the other journal.
                pass
        print(json.dumps(failure), file=sys.stderr, flush=True)
        raise
    finally:
        reserve.close()
        for number, handler in previous_handlers.items():
            signal.signal(number, handler)
        lock.close()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--state-dir", type=Path, required=True)
    parser.add_argument("--poll-seconds", type=float, default=10)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    try:
        result = observe(command, args.state_dir, args.poll_seconds)
    except (OSError, ValueError, RuntimeError):
        return 1
    return 0 if result["outcome"]["returncode"] == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
