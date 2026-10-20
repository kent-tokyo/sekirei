import importlib.util
import errno
import io
import json
import os
import signal
import sys
import tempfile
import threading
import time
from contextlib import redirect_stderr
from collections import deque
from pathlib import Path
from unittest.mock import patch


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


def test_client_status_observation_preserves_bounded_progress():
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "client-status.json"
        path.write_text(json.dumps({
            "schema": supervisor.CLIENT_STATUS_SCHEMA,
            "state": "stopped",
            "event": "max_games_reached",
            "timestamp_ms": 10_000,
            "pid": 42,
            "session_id": "42-1",
            "completed_attempts": 5,
            "max_games": 5,
            "terminal_stop_reason": "max_games_reached",
        }))
        observed = supervisor.observe_client_status(path, 42, now=10.0)
        assert observed["ok"]
        assert observed["completed_attempts"] == 5
        assert observed["max_games"] == 5
        assert observed["terminal_stop_reason"] == "max_games_reached"


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


def test_one_shot_records_interrupted_zero_game_session_and_never_respawns():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        state_dir = root / "state"
        status_file = root / "client-status.json"
        child_runs = root / "child-runs.txt"
        code = (
            "import json, os, pathlib, time; "
            f"runs=pathlib.Path({str(child_runs)!r}); "
            "runs.write_text(runs.read_text()+'x' if runs.exists() else 'x'); "
            f"p=pathlib.Path({str(status_file)!r}); "
            "p.write_text(json.dumps({'schema':'sekirei.csa-runtime-status.v1',"
            "'state':'waiting_for_game','event':None,'timestamp_ms':int(time.time()*1000),"
            "'pid':os.getpid(),'session_id':'session-1','active_game_id':None,"
            "'completed_attempts':0,'max_games':10,'terminal_stop_reason':None})); "
            "time.sleep(0.08); raise SystemExit(7)"
        )
        result = supervisor.run(
            [sys.executable, "-c", code], state_dir, root / "stop",
            max_restarts=5, restart_window=60.0, restart_delay=0.0, poll_seconds=0.01,
            client_status_file=status_file, client_status_max_age=2.0, one_shot=True,
        )
        assert result == 0
        completion = json.loads((state_dir / "one-shot-complete.json").read_text())
        assert completion["terminal_reason"] == "child_exited_before_max_games"
        assert completion["completed_attempts"] == 0
        assert completion["max_games"] == 10
        assert completion["child_exit_code"] == 7
        assert child_runs.read_text() == "x"

        repeated = supervisor.run(
            [sys.executable, "-c", code], state_dir, root / "stop",
            max_restarts=5, restart_window=60.0, restart_delay=0.0, poll_seconds=0.01,
            client_status_file=status_file, client_status_max_age=2.0, one_shot=True,
        )
        assert repeated == 0
        assert child_runs.read_text() == "x"
        events = [json.loads(line) for line in (state_dir / "events.jsonl").read_text().splitlines()]
        assert [event["kind"] for event in events].count("started") == 1
        assert events[-2]["kind"] == "one_shot_complete"
        assert events[-1]["kind"] == "one_shot_already_complete"


def test_one_shot_preserves_normal_max_games_terminal_reason():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        state_dir = root / "state"
        status_file = root / "client-status.json"
        code = (
            "import json, os, pathlib, time; "
            f"p=pathlib.Path({str(status_file)!r}); "
            "p.write_text(json.dumps({'schema':'sekirei.csa-runtime-status.v1',"
            "'state':'stopped','event':'max_games_reached','timestamp_ms':int(time.time()*1000),"
            "'pid':os.getpid(),'session_id':'session-1','active_game_id':None,"
            "'completed_attempts':3,'max_games':3,"
            "'terminal_stop_reason':'max_games_reached'})); time.sleep(0.08)"
        )
        result = supervisor.run(
            [sys.executable, "-c", code], state_dir, root / "stop",
            max_restarts=5, restart_window=60.0, restart_delay=0.0, poll_seconds=0.01,
            client_status_file=status_file, client_status_max_age=2.0, one_shot=True,
        )
        assert result == 0
        completion = json.loads((state_dir / "one-shot-complete.json").read_text())
        assert completion["terminal_reason"] == "max_games_reached"
        assert completion["completed_attempts"] == 3


def test_one_shot_preserves_reported_run_error_terminal_reason():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        state_dir = root / "state"
        status_file = root / "client-status.json"
        code = (
            "import json, os, pathlib, time; "
            f"p=pathlib.Path({str(status_file)!r}); "
            "p.write_text(json.dumps({'schema':'sekirei.csa-runtime-status.v1',"
            "'state':'client_error','event':'run_error','timestamp_ms':int(time.time()*1000),"
            "'pid':os.getpid(),'session_id':'session-1','active_game_id':None,"
            "'completed_attempts':3,'max_games':10,'terminal_stop_reason':'run_error'})); "
            "time.sleep(0.08); raise SystemExit(1)"
        )
        result = supervisor.run(
            [sys.executable, "-c", code], state_dir, root / "stop",
            max_restarts=5, restart_window=60.0, restart_delay=0.0, poll_seconds=0.01,
            client_status_file=status_file, client_status_max_age=2.0, one_shot=True,
        )
        assert result == 0
        completion = json.loads((state_dir / "one-shot-complete.json").read_text())
        assert completion["terminal_reason"] == "run_error"
        assert completion["completed_attempts"] == 3
        assert completion["max_games"] == 10
        assert completion["child_exit_code"] == 1
        events = [json.loads(line) for line in (state_dir / "events.jsonl").read_text().splitlines()]
        assert [event["kind"] for event in events].count("started") == 1
        assert all(event["kind"] != "restart_scheduled" for event in events)
        assert events[-1]["kind"] == "one_shot_complete"


def test_one_shot_idle_timeout_keeps_enospc_evidence_and_never_replays():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        state_dir = root / "state"
        status_file = root / "client-status.json"
        child_runs = root / "child-runs.txt"
        code = (
            "import json, os, pathlib, time; "
            f"p=pathlib.Path({str(status_file)!r}); "
            f"runs=pathlib.Path({str(child_runs)!r}); runs.write_text('x'); "
            "status={'schema':'sekirei.csa-runtime-status.v1',"
            "'state':'waiting_for_game','event':None,'timestamp_ms':int(time.time()*1000),"
            "'pid':os.getpid(),'session_id':'session-1','active_game_id':None,"
            "'completed_attempts':3,'max_games':10,'terminal_stop_reason':None}; "
            "p.write_text(json.dumps(status)); time.sleep(0.2); "
            "status.update({'state':'client_error','event':'between_game_receive_timeout',"
            "'timestamp_ms':int(time.time()*1000),"
            "'terminal_stop_reason':'between_game_receive_timeout',"
            "'between_game_receive_error_kind':'WouldBlock',"
            "'between_game_receive_os_error':35,"
            "'between_game_receive_idle_ms':2400000}); "
            "p.write_text(json.dumps(status)); time.sleep(0.2); raise SystemExit(1)"
        )
        real_open = Path.open
        real_write_json = supervisor.write_json

        def fail_event_journal_after_timeout(path, *args, **kwargs):
            if path.name == "events.jsonl" and args[:1] == ("a",) and status_file.exists():
                try:
                    status = json.loads(status_file.read_text())
                except json.JSONDecodeError:
                    status = {}
                if status.get("terminal_stop_reason") == "between_game_receive_timeout":
                    raise OSError(errno.ENOSPC, "injected event journal failure")
            return real_open(path, *args, **kwargs)

        def fail_state_on_transition(path, value):
            if path.name == "state.json" and value.get("kind") == "client_status_changed":
                raise OSError(errno.ENOSPC, "injected state failure")
            return real_write_json(path, value)

        with patch.object(Path, "open", fail_event_journal_after_timeout), \
                patch.object(supervisor, "write_json", fail_state_on_transition):
            result = supervisor.run(
                [sys.executable, "-c", code], state_dir, root / "stop",
                max_restarts=5, restart_window=60.0, restart_delay=0.0,
                poll_seconds=0.01, client_status_file=status_file,
                client_status_max_age=2.0, one_shot=True,
            )
        assert result == 0
        completion = json.loads((state_dir / "one-shot-complete.json").read_text())
        assert completion["terminal_reason"] == "between_game_receive_timeout"
        assert completion["completed_attempts"] == 3
        assert completion["max_games"] == 10
        assert completion["child_exit_code"] == 1
        assert completion["incomplete_game"] is None
        assert any("events.jsonl (client_status_changed): errno=28" in error
                   for error in completion["persistence_errors"])
        assert any("state.json (client_status_changed): errno=28" in error
                   for error in completion["persistence_errors"])
        status = json.loads(status_file.read_text())
        assert status["between_game_receive_error_kind"] == "WouldBlock"
        assert status["between_game_receive_os_error"] == 35
        assert status["between_game_receive_idle_ms"] == 2400000
        assert status["active_game_id"] is None
        assert child_runs.read_text() == "x"

        with patch.object(supervisor.subprocess, "Popen",
                          side_effect=AssertionError("child replayed")):
            repeated = supervisor.run(
                [sys.executable, "-c", code], state_dir, root / "stop",
                max_restarts=5, restart_window=60.0, restart_delay=0.0,
                poll_seconds=0.01, client_status_file=status_file, one_shot=True,
            )
        assert repeated == 0
        assert child_runs.read_text() == "x"


def test_one_shot_persists_mid_game_child_failure_without_counting_incomplete_game():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        state_dir = root / "state"
        status_file = root / "client-status.json"
        manifest_file = root / "run-manifest.json"
        record_dir = root / "records"
        record_dir.mkdir()
        partial_record = record_dir / "live-game.csa"
        partial_record.write_text("V2.2\nN+sekirei\nN-opponent\n+7776FU\n", encoding="utf-8")
        manifest_file.write_text(json.dumps({
            "schema": "sekirei.csa-run-manifest.v1",
            "status": "active",
            "completed_attempts": 2,
            "max_games": 10,
            "games": [{"game_id": "completed-1"}, {"game_id": "completed-2"}],
        }), encoding="utf-8")
        code = (
            "import json, os, pathlib, time; "
            f"p=pathlib.Path({str(status_file)!r}); "
            "p.write_text(json.dumps({'schema':'sekirei.csa-runtime-status.v1',"
            "'state':'in_game','event':'start','timestamp_ms':int(time.time()*1000),"
            "'pid':os.getpid(),'session_id':'session-live','active_game_id':'live-game',"
            "'completed_attempts':2,'max_games':10,'terminal_stop_reason':None})); "
            "time.sleep(0.08); raise SystemExit(3)"
        )
        result = supervisor.run(
            [sys.executable, "-c", code, "--run-manifest", str(manifest_file)],
            state_dir,
            root / "stop",
            max_restarts=5,
            restart_window=60.0,
            restart_delay=0.0,
            poll_seconds=0.01,
            client_status_file=status_file,
            client_status_max_age=2.0,
            one_shot=True,
        )
        assert result == 0

        status = json.loads(status_file.read_text())
        assert status["state"] == "client_error"
        assert status["event"] == "child_exit"
        assert status["completed_attempts"] == 2
        assert status["child_exit_code"] == 3
        assert status["incomplete_game"] == {
            "game_id": "live-game",
            "classification": "incomplete",
            "counted": False,
            "last_client_state": "in_game",
        }

        manifest = json.loads(manifest_file.read_text())
        assert manifest["status"] == "failed"
        assert manifest["completed_attempts"] == 2
        assert [game["game_id"] for game in manifest["games"]] == ["completed-1", "completed-2"]
        assert manifest["incomplete_game"]["game_id"] == "live-game"
        assert manifest["incomplete_game"]["counted"] is False

        completion = json.loads((state_dir / "one-shot-complete.json").read_text())
        assert completion["completed_attempts"] == 2
        assert completion["child_exit_code"] == 3
        assert completion["incomplete_game"]["classification"] == "incomplete"
        assert completion["persistence_errors"] == []
        assert partial_record.read_text(encoding="utf-8").endswith("+7776FU\n")
        events = [json.loads(line) for line in (state_dir / "events.jsonl").read_text().splitlines()]
        assert all(event["kind"] != "restart_scheduled" for event in events)
        assert events[-1]["kind"] == "one_shot_complete"


def _run_faulted_mid_game_child(root: Path) -> tuple[int, Path, Path, Path]:
    state_dir = root / "state"
    status_file = root / "client-status.json"
    manifest_file = root / "run-manifest.json"
    child_runs = root / "child-runs.txt"
    manifest_file.write_text(json.dumps({
        "schema": "sekirei.csa-run-manifest.v1", "status": "active",
        "completed_attempts": 2, "max_games": 10,
        "games": [{"game_id": "completed-1"}, {"game_id": "completed-2"}],
    }))
    code = (
        "import json, os, pathlib, time; "
        f"p=pathlib.Path({str(status_file)!r}); "
        f"runs=pathlib.Path({str(child_runs)!r}); "
        "runs.write_text(runs.read_text()+'x' if runs.exists() else 'x'); "
        "p.write_text(json.dumps({'schema':'sekirei.csa-runtime-status.v1',"
        "'state':'in_game','event':'start','timestamp_ms':int(time.time()*1000),"
        "'pid':os.getpid(),'session_id':'session-live','active_game_id':'live-game',"
        "'completed_attempts':2,'max_games':10,'terminal_stop_reason':None})); "
        "time.sleep(0.08); raise SystemExit(3)"
    )
    command = [sys.executable, "-c", code, "--run-manifest", str(manifest_file)]
    result = supervisor.run(
        command, state_dir, root / "stop", max_restarts=5,
        restart_window=60.0, restart_delay=0.0, poll_seconds=0.01,
        client_status_file=status_file, client_status_max_age=2.0, one_shot=True,
    )
    return result, state_dir, status_file, manifest_file


def _assert_faulted_mid_game_terminal(root: Path, failed_destination: str) -> None:
    state_dir = root / "state"
    status = json.loads((root / "client-status.json").read_text())
    manifest = json.loads((root / "run-manifest.json").read_text())
    completion = json.loads((state_dir / "one-shot-complete.json").read_text())
    assert status["state"] == "client_error"
    assert status["completed_attempts"] == 2
    assert status["child_exit_code"] == 3
    assert status["incomplete_game"]["counted"] is False
    assert manifest["status"] == "failed"
    assert manifest["completed_attempts"] == 2
    assert len(manifest["games"]) == 2
    assert manifest["incomplete_game"]["counted"] is False
    assert completion["completed_attempts"] == 2
    assert completion["max_games"] == 10
    assert completion["child_exit_code"] == 3
    assert completion["terminal_reason"] == "child_exited_before_max_games"
    assert completion["incomplete_game"]["counted"] is False
    assert any(failed_destination in error and "errno=28" in error
               for error in completion["persistence_errors"])
    assert (root / "child-runs.txt").read_text() == "x"
    # A durable receipt prevents a second child, even with an event write fault.
    assert supervisor.run(
        [sys.executable, "-c", "raise SystemExit(99)"], state_dir,
        root / "stop", max_restarts=5, restart_window=60.0,
        restart_delay=0.0, poll_seconds=0.01, one_shot=True,
    ) == 0
    assert (root / "child-runs.txt").read_text() == "x"


def test_one_shot_finalizes_after_exited_state_snapshot_failure():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        real_write_json = supervisor.write_json

        def fail_exited_snapshot(path, value):
            if path.name == "state.json" and value.get("kind") == "exited":
                raise OSError(errno.ENOSPC, "injected state snapshot failure")
            return real_write_json(path, value)

        with patch.object(supervisor, "write_json", fail_exited_snapshot):
            result, _, _, _ = _run_faulted_mid_game_child(root)
        assert result == 0
        _assert_faulted_mid_game_terminal(root, "state.json (exited)")


def test_one_shot_finalizes_after_event_journal_failure():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        real_open = Path.open

        def fail_event_journal(path, *args, **kwargs):
            if path.name == "events.jsonl" and args[:1] == ("a",):
                raise OSError(errno.ENOSPC, "injected journal failure")
            return real_open(path, *args, **kwargs)

        with patch.object(Path, "open", fail_event_journal):
            result, _, _, _ = _run_faulted_mid_game_child(root)
        assert result == 0
        _assert_faulted_mid_game_terminal(root, "events.jsonl")


def test_one_shot_reports_when_all_terminal_destinations_fail():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        real_open = Path.open
        real_write_json = supervisor.write_json

        def fail_event_journal(path, *args, **kwargs):
            if path.name == "events.jsonl" and args[:1] == ("a",):
                raise OSError(errno.ENOSPC, "injected journal failure")
            return real_open(path, *args, **kwargs)

        def fail_json_writes(path, value):
            if path.name == "one-shot-started.json":
                return real_write_json(path, value)
            raise OSError(errno.ENOSPC, f"injected {path.name} failure")

        fallback = io.StringIO()
        with patch.object(Path, "open", fail_event_journal), \
                patch.object(supervisor, "write_json", fail_json_writes), \
                redirect_stderr(fallback):
            result, state_dir, status_file, manifest_file = _run_faulted_mid_game_child(root)
        assert result == 1
        assert not (state_dir / "one-shot-complete.json").exists()
        assert (state_dir / "one-shot-started.json").exists()
        assert json.loads(status_file.read_text())["state"] == "in_game"
        assert json.loads(manifest_file.read_text())["status"] == "active"
        assert (root / "child-runs.txt").read_text() == "x"
        assert "events.jsonl" in fallback.getvalue()
        assert "one-shot-complete.json" in fallback.getvalue()
        assert "errno=28" in fallback.getvalue()


def test_missing_receipt_does_not_replay_a_terminal_child():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        real_write_json = supervisor.write_json

        def fail_receipt(path, value):
            if path.name == "one-shot-complete.json":
                raise OSError(errno.ENOSPC, "injected receipt failure")
            return real_write_json(path, value)

        with patch.object(supervisor, "write_json", fail_receipt):
            result, state_dir, status_file, manifest_file = _run_faulted_mid_game_child(root)
        assert result == 1
        assert not (state_dir / "one-shot-complete.json").exists()
        assert json.loads(status_file.read_text())["child_exit_code"] == 3
        assert json.loads(manifest_file.read_text())["child_exit_code"] == 3
        command = [sys.executable, "-c", "raise SystemExit(99)",
                   "--run-manifest", str(manifest_file)]
        with patch.object(supervisor.subprocess, "Popen",
                          side_effect=AssertionError("child replayed")):
            repeated = supervisor.run(
                command, state_dir, root / "stop", max_restarts=5,
                restart_window=60.0, restart_delay=0.0, poll_seconds=0.01,
                client_status_file=status_file, one_shot=True,
            )
        assert repeated == 1
        assert (root / "child-runs.txt").read_text() == "x"


def test_missing_receipt_does_not_replay_a_successful_bounded_child():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        state_dir = root / "state"
        status_file = root / "client-status.json"
        child_runs = root / "child-runs.txt"
        code = (
            "import json, os, pathlib, time; "
            f"p=pathlib.Path({str(status_file)!r}); "
            f"runs=pathlib.Path({str(child_runs)!r}); "
            "runs.write_text(runs.read_text()+'x' if runs.exists() else 'x'); "
            "p.write_text(json.dumps({'schema':'sekirei.csa-runtime-status.v1',"
            "'state':'stopped','event':'max_games_reached',"
            "'timestamp_ms':int(time.time()*1000),'pid':os.getpid(),"
            "'session_id':'session-1','active_game_id':None,"
            "'completed_attempts':3,'max_games':3,"
            "'terminal_stop_reason':'max_games_reached'})); time.sleep(0.08)"
        )
        real_write_json = supervisor.write_json

        def fail_receipt(path, value):
            if path.name == "one-shot-complete.json":
                raise OSError(errno.ENOSPC, "injected receipt failure")
            return real_write_json(path, value)

        with patch.object(supervisor, "write_json", fail_receipt):
            result = supervisor.run(
                [sys.executable, "-c", code], state_dir, root / "stop",
                max_restarts=5, restart_window=60.0, restart_delay=0.0,
                poll_seconds=0.01, client_status_file=status_file, one_shot=True,
            )
        assert result == 1
        assert not (state_dir / "one-shot-complete.json").exists()
        assert (state_dir / "one-shot-started.json").exists()
        assert child_runs.read_text() == "x"
        with patch.object(supervisor.subprocess, "Popen",
                          side_effect=AssertionError("child replayed")):
            repeated = supervisor.run(
                [sys.executable, "-c", code], state_dir, root / "stop",
                max_restarts=5, restart_window=60.0, restart_delay=0.0,
                poll_seconds=0.01, client_status_file=status_file, one_shot=True,
            )
        assert repeated == 1
        assert child_runs.read_text() == "x"


def test_log_write_and_rotation_failures_close_pipe_and_report_errno():
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "child.log"
        real_open = Path.open

        class BrokenWriter:
            def __init__(self, stream):
                self.stream = stream

            def tell(self):
                return self.stream.tell()

            def write(self, _data):
                raise OSError(errno.ENOSPC, "injected write failure")

            def close(self):
                self.stream.close()

        def open_broken_writer(target, *args, **kwargs):
            stream = real_open(target, *args, **kwargs)
            return BrokenWriter(stream) if target == path else stream

        pipe = io.BytesIO(b"x" * 16384)
        errors = []
        with patch.object(Path, "open", open_broken_writer):
            supervisor._copy_to_rotating_log(pipe, path, 4096, errors)
        assert pipe.closed
        assert len(errors) == 1 and "errno=28" in errors[0]

        def fail_rotation(_path):
            raise OSError(errno.ENOSPC, "injected rotation failure")

        pipe = io.BytesIO(b"y" * 16384)
        errors = []
        with patch.object(supervisor, "_rotate_log", fail_rotation):
            supervisor._copy_to_rotating_log(pipe, path, 1, errors)
        assert pipe.closed
        assert len(errors) == 1 and "errno=28" in errors[0]


def test_failed_log_open_drains_large_child_output_and_marks_run_failed():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        state_dir = root / "state"
        child_pid = []
        result_holder = {}
        real_open = Path.open
        real_popen = supervisor.subprocess.Popen

        def fail_stderr_log(path, *args, **kwargs):
            if path.name == "stderr-0001.log":
                raise OSError(errno.ENOSPC, "injected stderr log failure")
            return real_open(path, *args, **kwargs)

        def record_child(*args, **kwargs):
            process = real_popen(*args, **kwargs)
            child_pid.append(process.pid)
            return process

        def supervise():
            try:
                result_holder["result"] = supervisor.run(
                    [sys.executable, "-c",
                     "import os, time; os.write(2, b'x' * 262144); time.sleep(0.2)"],
                    state_dir, root / "stop", max_restarts=5,
                    restart_window=60.0, restart_delay=0.0, poll_seconds=0.01,
                    one_shot=True,
                )
            except BaseException as error:
                result_holder["error"] = error

        with patch.object(Path, "open", fail_stderr_log), \
                patch.object(supervisor.subprocess, "Popen", record_child):
            worker = threading.Thread(target=supervise)
            worker.start()
            worker.join(timeout=3.0)
            blocked = worker.is_alive()
            if blocked and child_pid:
                os.killpg(child_pid[0], signal.SIGKILL)
            worker.join(timeout=3.0)
        assert not blocked, "supervisor blocked after log open failure"
        assert not worker.is_alive(), "fixture could not clean up synthetic child"
        assert "error" not in result_holder, result_holder
        assert result_holder["result"] == 0
        assert len(child_pid) == 1
        completion = json.loads((state_dir / "one-shot-complete.json").read_text())
        assert completion["terminal_reason"] == "log_persistence_failed"
        assert completion["child_exit_code"] == 0
        assert any("stderr-0001.log" in error and "errno=28" in error
                   for error in completion["logging_errors"])
        events = [json.loads(line) for line in (state_dir / "events.jsonl").read_text().splitlines()]
        assert [event["kind"] for event in events].count("started") == 1
        assert any(event["kind"] == "logging_failed" and "errno=28" in event["error"]
                   for event in events)
        assert all(event["kind"] != "restart_scheduled" for event in events)


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
    test_client_status_observation_rejects_wrong_schema_or_state()
    test_client_status_observation_preserves_bounded_progress()
    test_graceful_stop_requires_a_fresh_non_active_status()
    test_runtime_graceful_stop_waits_for_game_completion()
    test_runtime_observes_client_session_without_confusing_process_restart()
    test_runtime_suppresses_timestamp_only_updates_but_records_state_changes()
    test_child_logs_are_rotated_at_a_bounded_generation_size()
    test_state_snapshot_contract_names_restart_count()
    test_runtime_restarts_unexpected_exit_then_opens_circuit_breaker()
    test_one_shot_records_interrupted_zero_game_session_and_never_respawns()
    test_one_shot_preserves_normal_max_games_terminal_reason()
    test_one_shot_preserves_reported_run_error_terminal_reason()
    test_one_shot_idle_timeout_keeps_enospc_evidence_and_never_replays()
    test_one_shot_persists_mid_game_child_failure_without_counting_incomplete_game()
    test_one_shot_finalizes_after_exited_state_snapshot_failure()
    test_one_shot_finalizes_after_event_journal_failure()
    test_one_shot_reports_when_all_terminal_destinations_fail()
    test_missing_receipt_does_not_replay_a_terminal_child()
    test_missing_receipt_does_not_replay_a_successful_bounded_child()
    test_log_write_and_rotation_failures_close_pipe_and_report_errno()
    test_failed_log_open_drains_large_child_output_and_marks_run_failed()
    test_runtime_explicit_stop_before_start_does_not_spawn_child()
    test_runtime_rejects_a_second_supervisor_for_the_same_state_directory()
    test_runtime_normal_exit_is_not_restarted()
    test_runtime_start_failure_is_terminal_without_restart_loop()
    test_configuration_exit_codes_are_terminal_without_restart()
    test_stop_during_restart_delay_prevents_next_spawn()
    print("PASS")
