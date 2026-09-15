import importlib.util
import json
import sys
import tempfile
import threading
import time
from collections import deque
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("supervisor", ROOT / "scripts/floodgate_supervisor.py")
supervisor = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(supervisor)


def test_exit_classification():
    assert supervisor.exit_kind(0, False) == "normal_exit"
    assert supervisor.exit_kind(7, False) == "exit_7"
    assert supervisor.exit_kind(-15, False) == "signal_15"
    assert supervisor.exit_kind(7, True) == "explicit_stop"


def test_only_unexpected_exit_is_restartable_and_window_is_bounded():
    assert supervisor.restart_allowed("exit_1", deque(), 100.0, 600.0, 2)
    assert not supervisor.restart_allowed("normal_exit", deque(), 100.0, 600.0, 2)
    assert not supervisor.restart_allowed("explicit_stop", deque(), 100.0, 600.0, 2)
    assert not supervisor.restart_allowed("exit_2", deque(), 100.0, 600.0, 2)
    assert not supervisor.restart_allowed("exit_3", deque(), 100.0, 600.0, 2)
    assert not supervisor.restart_allowed("exit_64", deque(), 100.0, 600.0, 2)
    assert not supervisor.restart_allowed("exit_1", deque([99.0, 99.5]), 100.0, 600.0, 2)
    assert supervisor.restart_allowed("exit_1", deque([0.0]), 100.0, 10.0, 1)


def test_operational_state_mapping_is_stable():
    assert supervisor.operational_state("running") == "running"
    assert supervisor.operational_state("restart_scheduled") == "restarting"
    assert supervisor.operational_state("explicit_stop_before_start") == "stopped"
    assert supervisor.operational_state("unknown-event") == "unknown"


def test_client_status_observation_rejects_stale_or_wrong_pid():
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "client-status.json"
        path.write_text(json.dumps({
            "schema": "sekirei.csa-runtime-status.v1",
            "state": "in_game",
            "timestamp_ms": 10_000,
            "pid": 42,
            "session_id": "42-1",
            "active_game_id": "game-1",
        }))
        assert supervisor.observe_client_status(path, 99, now=10.0)["reason"] == "pid_mismatch"
        assert supervisor.observe_client_status(path, 42, now=200.0)["reason"] == "stale_or_future"


def test_client_status_observation_rejects_wrong_schema_or_state():
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "client-status.json"
        document = {
            "schema": "wrong.schema",
            "state": "in_game",
            "timestamp_ms": 10_000,
            "pid": 42,
            "session_id": "42-1",
        }
        path.write_text(json.dumps(document))
        assert supervisor.observe_client_status(path, 42, now=10.0)["reason"] == "schema_mismatch"
        document["schema"] = supervisor.CLIENT_STATUS_SCHEMA
        document["state"] = "unknown"
        path.write_text(json.dumps(document))
        assert supervisor.observe_client_status(path, 42, now=10.0)["reason"] == "unknown_state"


def test_graceful_stop_requires_a_fresh_non_active_status():
    active = {"ok": True, "state": "in_game", "active_game_id": "game-1"}
    stale = {"ok": False, "reason": "stale_or_future"}
    waiting = {"ok": True, "state": "waiting_for_game", "active_game_id": None}
    assert not supervisor.safe_to_stop_after_game(active)
    assert not supervisor.safe_to_stop_after_game(stale)
    assert supervisor.safe_to_stop_after_game(waiting)


def test_runtime_graceful_stop_waits_for_game_completion():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        state_dir = root / "state"
        status_file = root / "client-status.json"
        graceful_file = root / "stop-after-game"
        code = (
            "import json, os, time; "
            f"p={str(status_file)!r}; "
            "base={'schema':'sekirei.csa-runtime-status.v1','state':'in_game',"
            "'timestamp_ms':int(time.time()*1000),'pid':os.getpid(),"
            "'active_game_id':'game-1','session_id':'session-1'}; "
            "open(p,'w').write(json.dumps(base)); time.sleep(0.20); "
            "base.update({'state':'game_finished','active_game_id':None,"
            "'timestamp_ms':int(time.time()*1000)}); open(p,'w').write(json.dumps(base)); "
            "time.sleep(5)"
        )
        result_holder = {}

        def supervise():
            result_holder['result'] = supervisor.run(
                [sys.executable, '-c', code], state_dir, root / 'stop',
                max_restarts=0, restart_window=60.0, restart_delay=0.0, poll_seconds=0.01,
                client_status_file=status_file, client_status_max_age=2.0,
                stop_after_game_file=graceful_file,
            )

        worker = threading.Thread(target=supervise)
        worker.start()
        deadline = time.time() + 2.0
        while time.time() < deadline:
            if status_file.exists() and '"in_game"' in status_file.read_text():
                break
            time.sleep(0.01)
        else:
            raise AssertionError('client did not enter game state')
        graceful_file.write_text('finish current game\\n')
        time.sleep(0.08)
        assert worker.is_alive()
        worker.join(timeout=3.0)
        assert not worker.is_alive()
        assert result_holder['result'] == 0
        events = [json.loads(line) for line in (state_dir / 'events.jsonl').read_text().splitlines()]
        kinds = [event['kind'] for event in events]
        assert 'stop_after_game_pending' in kinds
        assert 'stop_after_game_complete' in kinds


def test_runtime_observes_client_session_without_confusing_process_restart():
    with tempfile.TemporaryDirectory() as directory:
        state_dir = Path(directory) / "state"
        stop_file = Path(directory) / "stop"
        status_file = Path(directory) / "client-status.json"
        code = (
            "import json, os, time; "
            f"p={str(status_file)!r}; "
            "base={'schema':'sekirei.csa-runtime-status.v1','state':'in_game',"
            "'timestamp_ms':int(time.time()*1000),'pid':os.getpid(),"
            "'active_game_id':'game-1','session_id':'session-1'}; "
            "open(p,'w').write(json.dumps(base)); time.sleep(0.08); "
            "base['timestamp_ms']=int(time.time()*1000); base['session_id']='session-2'; "
            "open(p,'w').write(json.dumps(base)); time.sleep(0.08)"
        )
        result = supervisor.run(
            [sys.executable, "-c", code], state_dir, stop_file,
            max_restarts=0, restart_window=60.0, restart_delay=0.0, poll_seconds=0.01,
            client_status_file=status_file, client_status_max_age=2.0,
        )
        assert result == 0
        events = [json.loads(line) for line in (state_dir / "events.jsonl").read_text().splitlines()]
        assert any(event["kind"] == "client_status_observed" and event["session_id"] == "session-1"
                   for event in events)
        assert any(event["kind"] == "client_session_changed" and event["session_id"] == "session-2"
                   for event in events)


def test_runtime_suppresses_timestamp_only_updates_but_records_state_changes():
    with tempfile.TemporaryDirectory() as directory:
        state_dir = Path(directory) / "state"
        stop_file = Path(directory) / "stop"
        status_file = Path(directory) / "client-status.json"
        code = (
            "import json, os, time; "
            f"p={str(status_file)!r}; "
            "base={'schema':'sekirei.csa-runtime-status.v1','state':'in_game',"
            "'timestamp_ms':int(time.time()*1000),'pid':os.getpid(),"
            "'active_game_id':'game-1','session_id':'session-1'}; "
            "open(p,'w').write(json.dumps(base)); time.sleep(0.08); "
            "base['timestamp_ms']=int(time.time()*1000); open(p,'w').write(json.dumps(base)); time.sleep(0.08); "
            "base['timestamp_ms']=int(time.time()*1000); base['state']='game_finished'; base['event']='win'; "
            "open(p,'w').write(json.dumps(base)); time.sleep(0.08)"
        )
        result = supervisor.run(
            [sys.executable, "-c", code], state_dir, stop_file,
            max_restarts=0, restart_window=60.0, restart_delay=0.0, poll_seconds=0.01,
            client_status_file=status_file, client_status_max_age=2.0,
        )
        assert result == 0
        events = [json.loads(line) for line in (state_dir / "events.jsonl").read_text().splitlines()]
        status_events = [event for event in events if event["kind"].startswith("client_status")]
        assert [event["kind"] for event in status_events].count("client_status_observed") == 1
        assert [event["kind"] for event in status_events].count("client_status_changed") == 1
        assert status_events[-1]["client_state"] == "game_finished"


def test_child_logs_are_rotated_at_a_bounded_generation_size():
    with tempfile.TemporaryDirectory() as directory:
        state_dir = Path(directory) / "state"
        code = "import sys; sys.stdout.write('o'*2500); sys.stderr.write('e'*2500); sys.stdout.flush(); sys.stderr.flush()"
        result = supervisor.run(
            [sys.executable, "-c", code], state_dir, Path(directory) / "stop",
            max_restarts=0, restart_window=60.0, restart_delay=0.0, poll_seconds=0.01,
            max_log_bytes=1000,
        )
        assert result == 0
        stdout_logs = sorted(state_dir.glob("stdout-0001.log*"))
        stderr_logs = sorted(state_dir.glob("stderr-0001.log*"))
        assert len(stdout_logs) >= 2
        assert len(stderr_logs) >= 2
        assert all(path.stat().st_size <= 1000 for path in stdout_logs)
        assert all(path.stat().st_size <= 1000 for path in stderr_logs)
        assert sum(path.stat().st_size for path in stdout_logs) == 2500
        assert sum(path.stat().st_size for path in stderr_logs) == 2500


def test_state_snapshot_contract_names_restart_count():
    # The runtime event writer owns the counter; this guards the public field
    # name used by an external monitor without requiring a child process.
    source = (ROOT / "scripts/floodgate_supervisor.py").read_text(encoding="utf-8")
    assert '"restart_count": restart_count' in source


def test_runtime_restarts_unexpected_exit_then_opens_circuit_breaker():
    with tempfile.TemporaryDirectory() as directory:
        state_dir = Path(directory) / "state"
        stop_file = Path(directory) / "stop"
        code = "import sys; sys.exit(7)"
        result = supervisor.run(
            [sys.executable, "-c", code], state_dir, stop_file,
            max_restarts=1, restart_window=60.0, restart_delay=0.0, poll_seconds=0.01,
        )
        assert result == 1
        events = [json.loads(line) for line in (state_dir / "events.jsonl").read_text().splitlines()]
        assert [event["kind"] for event in events].count("started") == 2
        assert events[-1]["kind"] == "circuit_breaker"
        exits = [event for event in events if event["kind"] == "exited"]
        assert all(event["exit_kind"] == "exit_7" for event in exits)
        assert all(event["child_exit_code"] == 7 for event in exits)
        running = [event for event in events if event["kind"] == "running"]
        assert all(isinstance(event["child_started_at"], float) for event in running)
        assert [event["child_started_at"] for event in running] == [
            event["child_started_at"] for event in exits
        ]
        assert all(Path(event["stdout_path"]).is_file() for event in running)
        assert all(Path(event["stderr_path"]).is_file() for event in exits)


def test_runtime_explicit_stop_before_start_does_not_spawn_child():
    with tempfile.TemporaryDirectory() as directory:
        state_dir = Path(directory) / "state"
        stop_file = Path(directory) / "stop"
        stop_file.write_text("stop\n")
        result = supervisor.run(
            [sys.executable, "-c", "raise SystemExit(99)"], state_dir, stop_file,
            max_restarts=5, restart_window=60.0, restart_delay=0.0, poll_seconds=0.01,
        )
        assert result == 0
        events = [json.loads(line) for line in (state_dir / "events.jsonl").read_text().splitlines()]
        assert [event["kind"] for event in events] == ["explicit_stop_before_start"]


def test_runtime_rejects_a_second_supervisor_for_the_same_state_directory():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        state_dir = root / "state"
        stop_file = root / "stop"
        result_holder = {}

        def supervise():
            result_holder["first"] = supervisor.run(
                [sys.executable, "-c", "import time; time.sleep(2)"], state_dir, stop_file,
                max_restarts=0, restart_window=60.0, restart_delay=0.0, poll_seconds=0.01,
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
            raise AssertionError("first supervisor did not reach running state")

        assert supervisor.run(
            [sys.executable, "-c", "raise SystemExit(99)"], state_dir, stop_file,
            max_restarts=0, restart_window=60.0, restart_delay=0.0, poll_seconds=0.01,
        ) == 1
        assert len(list(state_dir.glob("stdout-*.log"))) == 1
        stop_file.write_text("stop\n")
        worker.join(timeout=3.0)
        assert not worker.is_alive()
        assert result_holder["first"] == 0


def test_runtime_normal_exit_is_not_restarted():
    with tempfile.TemporaryDirectory() as directory:
        state_dir = Path(directory) / "state"
        result = supervisor.run(
            [sys.executable, "-c", "raise SystemExit(0)"], state_dir, Path(directory) / "stop",
            max_restarts=5, restart_window=60.0, restart_delay=0.0, poll_seconds=0.01,
        )
        assert result == 0
        events = [json.loads(line) for line in (state_dir / "events.jsonl").read_text().splitlines()]
        assert [event["kind"] for event in events].count("started") == 1
        assert events[-1]["exit_kind"] == "normal_exit"


def test_runtime_start_failure_is_terminal_without_restart_loop():
    with tempfile.TemporaryDirectory() as directory:
        state_dir = Path(directory) / "state"
        result = supervisor.run(
            [str(Path(directory) / "does-not-exist")], state_dir, Path(directory) / "stop",
            max_restarts=5, restart_window=60.0, restart_delay=0.0, poll_seconds=0.01,
        )
        assert result == 1
        events = [json.loads(line) for line in (state_dir / "events.jsonl").read_text().splitlines()]
        assert [event["kind"] for event in events] == ["started", "start_failed"]


def test_configuration_exit_codes_are_terminal_without_restart():
    for exit_code in (2, 3, 64):
        with tempfile.TemporaryDirectory() as directory:
            state_dir = Path(directory) / "state"
            result = supervisor.run(
                [sys.executable, "-c", f"raise SystemExit({exit_code})"], state_dir,
                Path(directory) / "stop", max_restarts=5, restart_window=60.0,
                restart_delay=0.0, poll_seconds=0.01,
            )
            assert result == 1
            events = [json.loads(line) for line in (state_dir / "events.jsonl").read_text().splitlines()]
            assert [event["kind"] for event in events] == [
                "started", "running", "exited", "terminal_error"
            ]
            assert events[-1]["exit_kind"] == f"exit_{exit_code}"


def test_stop_during_restart_delay_prevents_next_spawn():
    with tempfile.TemporaryDirectory() as directory:
        state_dir = Path(directory) / "state"
        stop_file = Path(directory) / "stop"
        result_holder = {}

        def supervise():
            result_holder["result"] = supervisor.run(
                [sys.executable, "-c", "raise SystemExit(7)"], state_dir, stop_file,
                max_restarts=5, restart_window=60.0, restart_delay=0.2, poll_seconds=0.01,
            )

        worker = threading.Thread(target=supervise)
        worker.start()
        events_path = state_dir / "events.jsonl"
        deadline = time.time() + 2.0
        while time.time() < deadline:
            if events_path.exists() and "restart_scheduled" in events_path.read_text():
                break
            time.sleep(0.01)
        else:
            raise AssertionError("restart was not scheduled")
        stop_file.write_text("stop\n")
        worker.join(timeout=2.0)
        assert not worker.is_alive()
        assert result_holder["result"] == 0
        events = [json.loads(line) for line in events_path.read_text().splitlines()]
        assert [event["kind"] for event in events].count("running") == 1
        assert events[-1]["kind"] == "explicit_stop_before_start"


if __name__ == "__main__":
    test_exit_classification()
    test_only_unexpected_exit_is_restartable_and_window_is_bounded()
    test_client_status_observation_rejects_stale_or_wrong_pid()
    test_graceful_stop_requires_a_fresh_non_active_status()
    test_runtime_graceful_stop_waits_for_game_completion()
    test_runtime_observes_client_session_without_confusing_process_restart()
    test_runtime_suppresses_timestamp_only_updates_but_records_state_changes()
    test_child_logs_are_rotated_at_a_bounded_generation_size()
    test_runtime_restarts_unexpected_exit_then_opens_circuit_breaker()
    test_runtime_explicit_stop_before_start_does_not_spawn_child()
    test_runtime_rejects_a_second_supervisor_for_the_same_state_directory()
    test_runtime_normal_exit_is_not_restarted()
    test_runtime_start_failure_is_terminal_without_restart_loop()
    test_configuration_exit_codes_are_terminal_without_restart()
    test_stop_during_restart_delay_prevents_next_spawn()
    print("PASS")
