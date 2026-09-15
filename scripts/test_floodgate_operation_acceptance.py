"""Offline acceptance fixture for the Floodgate supervisor operation path."""
import importlib.util
import json
import sys
import subprocess
import tempfile
import threading
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("supervisor", ROOT / "scripts/floodgate_supervisor.py")
supervisor = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(supervisor)


def test_same_entrypoint_captures_two_client_generations_and_bounds_restart():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        state_dir = root / "state"
        status_file = root / "client-status.json"
        code = (
            "import json, os, sys, time; "
            "path=sys.argv[1]; "
            "document={'schema':'sekirei.csa-runtime-status.v1',"
            "'state':'in_game','timestamp_ms':int(time.time()*1000),"
            "'pid':os.getpid(),'active_game_id':'acceptance-game',"
            "'session_id':str(os.getpid())+'-'+str(time.time_ns())}; "
            "open(path,'w').write(json.dumps(document)); "
            "time.sleep(0.08); sys.exit(7)"
        )
        result = supervisor.run(
            [sys.executable, "-c", code, str(status_file)], state_dir, root / "stop",
            max_restarts=1, restart_window=60.0, restart_delay=0.0, poll_seconds=0.01,
            client_status_file=status_file, client_status_max_age=2.0, max_log_bytes=1024,
        )
        assert result == 1
        events = [json.loads(line) for line in (state_dir / "events.jsonl").read_text().splitlines()]
        assert [event["kind"] for event in events].count("started") == 2
        assert [event["kind"] for event in events].count("running") == 2
        assert [event["kind"] for event in events].count("exited") == 2
        observed = [event for event in events if event["kind"] == "client_status_observed"]
        assert observed
        assert {event["attempt"] for event in observed} == {1, 2}
        assert events[-1]["kind"] == "circuit_breaker"
        assert all(event["child_exit_code"] == 7 for event in events if event["kind"] == "exited")
        assert all(path.stat().st_size <= 1024 for path in state_dir.glob("stdout-*.log*"))
        assert all(path.stat().st_size <= 1024 for path in state_dir.glob("stderr-*.log*"))


def test_same_entrypoint_keeps_explicit_stop_and_configuration_error_distinct():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        state_dir = root / "stop-state"
        stop_file = root / "stop"
        result_holder = {}

        def supervise():
            result_holder["result"] = supervisor.run(
                [sys.executable, "-c", "import time; time.sleep(2)"], state_dir, stop_file,
                max_restarts=5, restart_window=60.0, restart_delay=0.0, poll_seconds=0.01,
            )

        worker = threading.Thread(target=supervise)
        worker.start()
        events_path = state_dir / "events.jsonl"
        deadline = time.time() + 2.0
        while time.time() < deadline:
            if events_path.exists() and '"kind": "running"' in events_path.read_text():
                break
            time.sleep(0.01)
        else:
            raise AssertionError("child did not reach running state")
        stop_file.write_text("stop\n")
        worker.join(timeout=3.0)
        assert not worker.is_alive()
        assert result_holder["result"] == 0
        events = [json.loads(line) for line in events_path.read_text().splitlines()]
        assert [event["kind"] for event in events].count("running") == 1
        assert any(event["kind"] == "stop_requested" for event in events)
        assert all(event["kind"] != "restart_scheduled" for event in events)

        terminal_state = root / "terminal-state"
        result = supervisor.run(
            [sys.executable, "-c", "raise SystemExit(3)"], terminal_state, root / "terminal-stop",
            max_restarts=5, restart_window=60.0, restart_delay=0.0, poll_seconds=0.01,
        )
        assert result == 1
        terminal_events = [
            json.loads(line) for line in (terminal_state / "events.jsonl").read_text().splitlines()
        ]
        assert [event["kind"] for event in terminal_events].count("running") == 1
        assert terminal_events[-1]["kind"] == "terminal_error"
        assert all(event["kind"] != "circuit_breaker" for event in terminal_events)

        record_failure_state = root / "record-failure-state"
        record_failure_status = root / "record-failure-status.json"
        record_failure_code = (
            "import json, os, sys, time; "
            "path=sys.argv[1]; "
            "document={'schema':'sekirei.csa-runtime-status.v1',"
            "'state':'client_error','event':'record_initialization_failed',"
            "'timestamp_ms':int(time.time()*1000),'pid':os.getpid(),"
            "'active_game_id':'record-failure','session_id':'fixture-session'}; "
            "open(path,'w').write(json.dumps(document)); "
            "time.sleep(0.08); "
            "sys.exit(3)"
        )
        result = supervisor.run(
            [sys.executable, "-c", record_failure_code, str(record_failure_status)],
            record_failure_state,
            root / "record-failure-stop",
            max_restarts=5,
            restart_window=60.0,
            restart_delay=0.0,
            poll_seconds=0.01,
            client_status_file=record_failure_status,
            client_status_max_age=2.0,
        )
        assert result == 1
        record_failure_events = [
            json.loads(line)
            for line in (record_failure_state / "events.jsonl").read_text().splitlines()
        ]
        assert any(
            event["kind"] == "client_status_observed"
            and event["client_state"] == "client_error"
            and event["client_event"] == "record_initialization_failed"
            for event in record_failure_events
        )
        assert record_failure_events[-1]["kind"] == "terminal_error"


def test_communication_failure_stop_during_restart_delay_prevents_respawn():
    """A retryable client failure must not defeat an explicit stop request."""
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        state_dir = root / "communication-state"
        attempt_file = root / "attempts"
        stop_file = root / "stop"
        code = (
            "import pathlib, sys, time; "
            "path=pathlib.Path(sys.argv[1]); "
            "count=int(path.read_text()) if path.exists() else 0; "
            "path.write_text(str(count+1)); "
            "time.sleep(0.06); "
            "sys.exit(1)"
        )

        result_holder = {}

        def supervise():
            result_holder["result"] = supervisor.run(
                [sys.executable, "-c", code, str(attempt_file)], state_dir, stop_file,
                max_restarts=5, restart_window=60.0, restart_delay=0.4, poll_seconds=0.01,
            )

        worker = threading.Thread(target=supervise)
        worker.start()
        events_path = state_dir / "events.jsonl"
        deadline = time.time() + 2.0
        while time.time() < deadline:
            if events_path.exists() and '"kind": "restart_scheduled"' in events_path.read_text():
                break
            time.sleep(0.01)
        else:
            raise AssertionError("retryable failure did not enter restart delay")

        stop_file.write_text("stop\n")
        worker.join(timeout=3.0)
        assert not worker.is_alive()
        assert result_holder["result"] == 0
        events = [json.loads(line) for line in events_path.read_text().splitlines()]
        assert [event["kind"] for event in events].count("started") == 1
        assert any(event["kind"] == "restart_scheduled" for event in events)
        assert events[-1]["kind"] == "explicit_stop_before_start"
        assert attempt_file.read_text() == "1"


def test_supervisor_signal_becomes_explicit_stop_and_reaps_child():
    """SIGTERM to the supervisor must not leave its isolated child running."""
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        state_dir = root / "signal-state"
        stop_file = root / "signal-stop"
        marker = root / "child-alive"
        child = (
            "import pathlib, sys, time; "
            "pathlib.Path(sys.argv[1]).write_text('started'); "
            "time.sleep(10)"
        )
        process = subprocess.Popen([
            sys.executable, str(ROOT / "scripts/floodgate_supervisor.py"),
            "--state-dir", str(state_dir), "--stop-file", str(stop_file),
            "--poll-seconds", "0.01", "--", sys.executable, "-c", child, str(marker),
        ])
        deadline = time.time() + 3.0
        while time.time() < deadline and not marker.exists():
            time.sleep(0.01)
        assert marker.exists()
        process.terminate()
        assert process.wait(timeout=3.0) == 0
        events = [json.loads(line) for line in (state_dir / "events.jsonl").read_text().splitlines()]
        assert any(event["kind"] == "stop_requested" for event in events)
        assert events[-1]["exit_kind"] == "explicit_stop"
        assert stop_file.exists()


if __name__ == "__main__":
    test_same_entrypoint_captures_two_client_generations_and_bounds_restart()
    test_same_entrypoint_keeps_explicit_stop_and_configuration_error_distinct()
    print("PASS")
