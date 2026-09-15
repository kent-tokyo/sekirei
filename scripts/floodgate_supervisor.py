#!/usr/bin/env python3
"""Bounded supervisor for a CSA client process.

The supervisor never uses a shell.  It restarts only unexpected non-zero
exits, gives an explicit stop file priority, and records operational state in
JSON/JSONL without copying credentials or environment variables.
"""
from __future__ import annotations

import argparse
import fcntl
import json
import os
import signal
import subprocess
import threading
import time
from collections import deque
from pathlib import Path

NON_RESTARTABLE_EXIT_KINDS = {"exit_2", "exit_3", "exit_64"}
CLIENT_STATUS_SCHEMA = "sekirei.csa-runtime-status.v1"
CLIENT_STATUS_STATES = {
    "connecting", "authenticated", "waiting_for_game", "in_game", "receiving",
    "game_finished", "connection_error", "client_error",
}


def exit_kind(returncode: int | None, stop_requested: bool) -> str:
    if stop_requested:
        return "explicit_stop"
    if returncode == 0:
        return "normal_exit"
    if returncode is None:
        return "not_started"
    if returncode < 0:
        return f"signal_{-returncode}"
    return f"exit_{returncode}"


def restart_allowed(kind: str, restart_times: deque[float], now: float, window: float, limit: int) -> bool:
    while restart_times and now - restart_times[0] > window:
        restart_times.popleft()
    return (
        kind not in {"normal_exit", "explicit_stop", *NON_RESTARTABLE_EXIT_KINDS}
        and len(restart_times) < limit
    )


def write_json(path: Path, value: dict) -> None:
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    temporary.replace(path)


def acquire_instance_lock(path: Path):
    """Keep one supervisor for a state directory, without deleting the lock."""
    lock = path.open("a+")
    try:
        fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        lock.close()
        return None
    return lock


def observe_client_status(path: Path, pid: int, now: float | None = None,
                         max_age_seconds: float = 120.0) -> dict[str, object]:
    """Read a client snapshot and verify that it belongs to the live child."""
    try:
        document = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        return {"ok": False, "reason": "missing"}
    except (OSError, json.JSONDecodeError):
        return {"ok": False, "reason": "invalid_json"}
    if not isinstance(document, dict):
        return {"ok": False, "reason": "not_an_object"}
    if document.get("schema") != CLIENT_STATUS_SCHEMA:
        return {"ok": False, "reason": "schema_mismatch"}
    if document.get("state") not in CLIENT_STATUS_STATES:
        return {"ok": False, "reason": "unknown_state"}
    if document.get("pid") != pid:
        return {"ok": False, "reason": "pid_mismatch"}
    session_id = document.get("session_id")
    if not isinstance(session_id, str) or not session_id:
        return {"ok": False, "reason": "missing_session_id"}
    timestamp_ms = document.get("timestamp_ms")
    if not isinstance(timestamp_ms, int) or isinstance(timestamp_ms, bool):
        return {"ok": False, "reason": "invalid_timestamp"}
    current = time.time() if now is None else now
    age = current - timestamp_ms / 1000.0
    if age > max_age_seconds or age < -5.0:
        return {"ok": False, "reason": "stale_or_future"}
    return {
        "ok": True,
        "session_id": session_id,
        "state": document.get("state"),
        "event": document.get("event"),
        "active_game_id": document.get("active_game_id"),
        "timestamp_ms": timestamp_ms,
    }


def safe_to_stop_after_game(observation: dict[str, object]) -> bool:
    """Honor a graceful stop only after a fresh non-active client snapshot."""
    return (bool(observation.get("ok"))
            and observation.get("state") not in {"in_game", "receiving"}
            and observation.get("active_game_id") is None)


def _rotate_log(path: Path, max_rotations: int = 3) -> None:
    for index in range(max_rotations - 1, 0, -1):
        older = path.with_name(f"{path.name}.{index}")
        newer = path.with_name(f"{path.name}.{index + 1}")
        if older.exists():
            older.replace(newer)
    if path.exists():
        path.replace(path.with_name(f"{path.name}.1"))


def _copy_to_rotating_log(pipe, path: Path, max_bytes: int) -> None:
    """Drain a child pipe without allowing one log generation to grow unbounded."""
    stream = path.open("ab")
    try:
        while True:
            chunk = pipe.read(8192)
            if not chunk:
                break
            offset = 0
            while offset < len(chunk):
                remaining = max_bytes - stream.tell()
                if remaining <= 0:
                    stream.close()
                    _rotate_log(path)
                    stream = path.open("ab")
                    remaining = max_bytes
                take = min(remaining, len(chunk) - offset)
                stream.write(chunk[offset:offset + take])
                stream.flush()
                offset += take
    finally:
        stream.close()
        pipe.close()


def operational_state(kind: str) -> str:
    return {
        "started": "starting",
        "running": "running",
        "stop_requested": "stopping",
        "exited": "exited",
        "restart_scheduled": "restarting",
        "circuit_breaker": "blocked",
        "terminal_error": "error",
        "client_status_observed": "running",
        "client_status_changed": "running",
        "client_session_changed": "running",
        "client_status_invalid": "running",
        "explicit_stop_before_start": "stopped",
        "stop_after_game_before_start": "stopped",
        "stop_after_game_pending": "stopping",
        "stop_after_game_complete": "stopping",
        "start_failed": "error",
    }.get(kind, "unknown")


def run(command: list[str], state_dir: Path, stop_file: Path, max_restarts: int,
        restart_window: float, restart_delay: float, poll_seconds: float,
        client_status_file: Path | None = None,
        client_status_max_age: float = 120.0,
        max_log_bytes: int = 10 * 1024 * 1024,
        stop_after_game_file: Path | None = None) -> int:
    if not command:
        raise ValueError("a command is required after --")
    state_dir.mkdir(parents=True, exist_ok=True)
    instance_lock = acquire_instance_lock(state_dir / "supervisor.lock")
    if instance_lock is None:
        return 1
    events_path = state_dir / "events.jsonl"
    state_path = state_dir / "state.json"
    restart_times: deque[float] = deque()
    restart_count = 0

    def event(kind: str, **fields: object) -> None:
        value = {
            "timestamp": time.time(),
            "kind": kind,
            "last_event": kind,
            "state": operational_state(kind),
            "restart_count": restart_count,
            **fields,
        }
        with events_path.open("a", encoding="utf-8") as stream:
            stream.write(json.dumps(value, ensure_ascii=False) + "\n")
        write_json(state_path, value)

    if stop_file.exists():
        event("explicit_stop_before_start", command=command)
        return 0
    if stop_after_game_file is not None and stop_after_game_file.exists():
        event("stop_after_game_before_start", command=command)
        return 0

    while True:
        if stop_file.exists():
            event("explicit_stop_before_start", command=command, restart_count=restart_count)
            return 0
        if stop_after_game_file is not None and stop_after_game_file.exists():
            event("stop_after_game_before_start", command=command, restart_count=restart_count)
            return 0
        attempt = restart_count + 1
        stdout_path = state_dir / f"stdout-{attempt:04d}.log"
        stderr_path = state_dir / f"stderr-{attempt:04d}.log"
        started = time.time()
        event("started", attempt=attempt, command=command, pid=None)
        try:
            process = subprocess.Popen(
                command, stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True
            )
        except OSError as exc:
            event("start_failed", attempt=attempt, error=str(exc), returncode=None)
            return 1
        assert process.stdout is not None and process.stderr is not None
        stdout_pump = threading.Thread(
            target=_copy_to_rotating_log, args=(process.stdout, stdout_path, max_log_bytes), daemon=True
        )
        stderr_pump = threading.Thread(
            target=_copy_to_rotating_log, args=(process.stderr, stderr_path, max_log_bytes), daemon=True
        )
        stdout_pump.start()
        stderr_pump.start()
        try:
            event(
                "running",
                attempt=attempt,
                pid=process.pid,
                child_started_at=started,
                stdout_path=str(stdout_path),
                stderr_path=str(stderr_path),
                client_status_file=str(client_status_file) if client_status_file else None,
            )
            stop_requested = False
            graceful_stop_pending = False
            observed_status: tuple[object, object, object, object] | None = None
            last_status_error: str | None = None
            while process.poll() is None:
                observation: dict[str, object] | None = None
                if client_status_file is not None:
                    observation = observe_client_status(
                        client_status_file, process.pid, max_age_seconds=client_status_max_age
                    )
                    if observation["ok"]:
                        session_id = str(observation["session_id"])
                        status_key = (
                            session_id,
                            observation["state"],
                            observation["event"],
                            observation["active_game_id"],
                        )
                        if observed_status is None:
                            event("client_status_observed", attempt=attempt, pid=process.pid,
                                  session_id=session_id,
                                  client_state=observation["state"],
                                  client_event=observation["event"],
                                  active_game_id=observation["active_game_id"])
                        elif observed_status != status_key:
                            previous_session, previous_state, previous_event, previous_game = observed_status
                            kind = "client_session_changed" if previous_session != session_id else "client_status_changed"
                            event(kind, attempt=attempt, pid=process.pid,
                                  previous_session_id=previous_session,
                                  previous_client_state=previous_state,
                                  previous_client_event=previous_event,
                                  previous_active_game_id=previous_game,
                                  session_id=session_id,
                                  client_state=observation["state"],
                                  client_event=observation["event"],
                                  active_game_id=observation["active_game_id"])
                        observed_status = status_key
                        last_status_error = None
                    elif observation["reason"] != last_status_error:
                        last_status_error = str(observation["reason"])
                        event("client_status_invalid", attempt=attempt, pid=process.pid,
                              reason=last_status_error)
                if stop_file.exists():
                    stop_requested = True
                    event("stop_requested", attempt=attempt, pid=process.pid)
                    try:
                        os.killpg(process.pid, signal.SIGTERM)
                    except ProcessLookupError:
                        pass
                    break
                if stop_after_game_file is not None and stop_after_game_file.exists():
                    if observation is not None and safe_to_stop_after_game(observation):
                        stop_requested = True
                        event("stop_after_game_complete", attempt=attempt, pid=process.pid,
                              client_state=observation["state"],
                              active_game_id=observation["active_game_id"])
                        try:
                            os.killpg(process.pid, signal.SIGTERM)
                        except ProcessLookupError:
                            pass
                        break
                    if not graceful_stop_pending:
                        graceful_stop_pending = True
                        event("stop_after_game_pending", attempt=attempt, pid=process.pid,
                              reason="awaiting_fresh_non_active_client_status")
                time.sleep(poll_seconds)
            if stop_requested:
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
            returncode = process.returncode
        finally:
            stdout_pump.join()
            stderr_pump.join()
        kind = exit_kind(returncode, stop_requested)
        event("exited", attempt=attempt, pid=process.pid, returncode=returncode,
              child_exit_code=returncode,
              child_started_at=started, exit_kind=kind,
              stdout_path=str(stdout_path), stderr_path=str(stderr_path),
              client_status_file=str(client_status_file) if client_status_file else None,
              elapsed_seconds=round(time.time() - started, 3))
        if kind in {"normal_exit", "explicit_stop"}:
            return 0
        if kind in NON_RESTARTABLE_EXIT_KINDS:
            event("terminal_error", attempt=attempt, pid=process.pid,
                  exit_kind=kind, child_exit_code=returncode)
            return 1
        now = time.time()
        if not restart_allowed(kind, restart_times, now, restart_window, max_restarts):
            event("circuit_breaker", attempt=attempt, restart_count=restart_count,
                  window_seconds=restart_window, max_restarts=max_restarts)
            return 1
        restart_times.append(now)
        restart_count += 1
        event("restart_scheduled", attempt=attempt, restart_count=restart_count,
              delay_seconds=restart_delay)
        time.sleep(restart_delay)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--state-dir", type=Path, required=True)
    parser.add_argument("--stop-file", type=Path, required=True)
    parser.add_argument("--max-restarts", type=int, default=5)
    parser.add_argument("--restart-window", type=float, default=600.0)
    parser.add_argument("--restart-delay", type=float, default=5.0)
    parser.add_argument("--poll-seconds", type=float, default=1.0)
    parser.add_argument("--client-status-file", type=Path, default=None)
    parser.add_argument("--client-status-max-age", type=float, default=120.0)
    parser.add_argument("--max-log-bytes", type=int, default=10 * 1024 * 1024)
    parser.add_argument("--stop-after-game-file", type=Path, default=None)
    parser.add_argument("command", nargs=argparse.REMAINDER, help="child command after --")
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if (args.max_restarts < 0 or args.restart_window <= 0 or args.restart_delay < 0
            or args.poll_seconds <= 0 or args.client_status_max_age < 0 or args.max_log_bytes <= 0):
        parser.error("restart limits and polling values are invalid")
    if args.stop_after_game_file is not None and args.client_status_file is None:
        parser.error("--stop-after-game-file requires --client-status-file")
    previous_handlers = {}

    def request_stop(_signum, _frame):
        # The child is deliberately placed in its own session.  Convert a
        # supervisor signal into the same durable stop request used by the
        # operator, so the run loop can terminate the child process group and
        # record an explicit stop instead of leaving an orphan behind.
        try:
            args.stop_file.parent.mkdir(parents=True, exist_ok=True)
            args.stop_file.touch(exist_ok=True)
        except OSError:
            # The run loop will report the resulting child outcome; signal
            # handlers must remain minimal and never raise into Python.
            pass

    for signal_number in (signal.SIGTERM, signal.SIGINT):
        previous_handlers[signal_number] = signal.signal(signal_number, request_stop)
    try:
        return run(command, args.state_dir, args.stop_file, args.max_restarts,
                   args.restart_window, args.restart_delay, args.poll_seconds,
                   args.client_status_file, args.client_status_max_age, args.max_log_bytes,
                   args.stop_after_game_file)
    except ValueError as exc:
        parser.error(str(exc))
        return 2
    finally:
        for signal_number, handler in previous_handlers.items():
            signal.signal(signal_number, handler)


if __name__ == "__main__":
    raise SystemExit(main())
