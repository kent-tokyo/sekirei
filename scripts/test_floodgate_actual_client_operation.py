#!/usr/bin/env python3
"""Run the actual CSA client under the offline supervisor fixture."""
from __future__ import annotations

import importlib.util
import json
import socket
import tempfile
import threading
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("supervisor", ROOT / "scripts/floodgate_supervisor.py")
supervisor = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(supervisor)
EVENT_SPEC = importlib.util.spec_from_file_location(
    "supervisor_events", ROOT / "scripts/validate_floodgate_supervisor_events.py"
)
supervisor_events = importlib.util.module_from_spec(EVENT_SPEC)
assert EVENT_SPEC.loader is not None
EVENT_SPEC.loader.exec_module(supervisor_events)
ANALYSIS_SPEC = importlib.util.spec_from_file_location(
    "analysis_validator", ROOT / "scripts/validate_analysis_record.py"
)
analysis_validator = importlib.util.module_from_spec(ANALYSIS_SPEC)
assert ANALYSIS_SPEC.loader is not None
ANALYSIS_SPEC.loader.exec_module(analysis_validator)


def test_actual_client_completes_one_offline_game():
    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    listener.listen(1)
    port = listener.getsockname()[1]
    server_error = []

    def serve():
        try:
            connection, _ = listener.accept()
            with connection, connection.makefile("rwb") as stream:
                assert stream.readline().decode().startswith("LOGIN test ")
                stream.write(b"LOGIN: test OK\n")
                stream.flush()
                assert stream.readline().decode().startswith("%%GAME offline-game")
                stream.write(
                    b"Game_ID:offline-game\nYour_Turn:+\nTotal_Time:1\nByoyomi:1\n"
                    b"END Game_Summary\nBEGIN Position\nPI\nEND Position\nSTART:offline-game\n"
                )
                stream.flush()
                assert stream.readline().decode().startswith("AGREE:offline-game")
                move = stream.readline().decode().strip()
                assert move.startswith("+")
                stream.write((move + ",T0\n#WIN\n").encode())
                stream.flush()
        except Exception as error:  # pragma: no cover - reported by the parent assertion
            server_error.append(error)
        finally:
            listener.close()

    server = threading.Thread(target=serve)
    server.start()
    with tempfile.TemporaryDirectory(prefix="sekirei-actual-client-") as directory:
        root = Path(directory)
        state_dir = root / "state"
        record_dir = root / "records"
        analysis_dir = root / "analysis"
        status_file = root / "client-status.json"
        run_manifest = root / "run-manifest.json"
        command = [
            str(ROOT / "target/debug/sekirei-csa"),
            "--server", "127.0.0.1", "--port", str(port),
            "--user", "test", "--password", "secret",
            "--eval", "material", "--depth", "1", "--hash", "1",
            "--game", "offline-game", "--record-dir", str(record_dir),
            "--analysis-dir", str(analysis_dir), "--run-manifest", str(run_manifest),
            "--status-file", str(status_file), "--root-candidates", "7g7f,2g2f",
        ]
        result = supervisor.run(
            command, state_dir, root / "stop", max_restarts=1, restart_window=60.0,
            restart_delay=0.0, poll_seconds=0.01, client_status_file=status_file,
            client_status_max_age=2.0, max_log_bytes=4096,
        )
        server.join(timeout=5.0)
        assert not server.is_alive()
        assert not server_error, server_error
        assert result == 0
        events = [json.loads(line) for line in (state_dir / "events.jsonl").read_text().splitlines()]
        assert supervisor_events.validate(events) == [], events
        assert [event["kind"] for event in events].count("started") == 1
        assert [event["kind"] for event in events].count("running") == 1
        assert events[-1]["kind"] == "exited"
        assert events[-1]["exit_kind"] == "normal_exit"
        assert any(event["kind"] == "client_status_observed" for event in events)
        status = json.loads(status_file.read_text())
        assert status["state"] == "game_finished"
        assert status["event"] == "win"
        manifest = json.loads(run_manifest.read_text())
        assert "game" in manifest, manifest
        assert manifest["game"]["game_id"] == "offline-game"
        assert manifest["game"]["initial_sfen"].endswith(" b - 1")
        assert len(list(record_dir.glob("*.csa"))) == 1
        analysis_path = next(analysis_dir.glob("*.jsonl"))
        analysis_lines = analysis_path.read_text(encoding="utf-8").splitlines()
        assert analysis_validator.validate_lines(analysis_lines) == [], analysis_lines
        search = json.loads(analysis_lines[1])
        assert len(search["root_candidates"]) == 2
        assert {candidate["move_csa"] for candidate in search["root_candidates"]} == {"+7776FU", "+2726FU"}


def test_actual_client_connection_loss_is_bounded_by_supervisor():
    """A real client EOF is retryable, but repeated EOFs must circuit-break."""
    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    listener.listen(2)
    port = listener.getsockname()[1]
    server_error = []

    def serve():
        try:
            for _ in range(2):
                connection, _ = listener.accept()
                with connection, connection.makefile("rwb") as stream:
                    assert stream.readline().decode().startswith("LOGIN test ")
                    stream.write(b"LOGIN: test OK\n")
                    stream.flush()
                    # Closing before %%GAME is a communication failure, not a game result.
        except Exception as error:  # pragma: no cover - reported by the parent assertion
            server_error.append(error)
        finally:
            listener.close()

    server = threading.Thread(target=serve)
    server.start()
    with tempfile.TemporaryDirectory(prefix="sekirei-client-eof-") as directory:
        root = Path(directory)
        state_dir = root / "state"
        status_file = root / "client-status.json"
        command = [
            str(ROOT / "target/debug/sekirei-csa"),
            "--server", "127.0.0.1", "--port", str(port),
            "--user", "test", "--password", "secret",
            "--eval", "material", "--depth", "1", "--hash", "1",
            "--game", "offline-eof", "--status-file", str(status_file),
        ]
        result = supervisor.run(
            command, state_dir, root / "stop", max_restarts=1, restart_window=60.0,
            restart_delay=0.0, poll_seconds=0.01, client_status_file=status_file,
            client_status_max_age=2.0, max_log_bytes=4096,
        )
        server.join(timeout=5.0)
        assert not server.is_alive()
        assert not server_error, server_error
        assert result == 1
        events = [json.loads(line) for line in (state_dir / "events.jsonl").read_text().splitlines()]
        assert supervisor_events.validate(events) == []
        assert [event["kind"] for event in events].count("started") == 2
        assert [event["kind"] for event in events].count("restart_scheduled") == 1
        assert events[-1]["kind"] == "circuit_breaker"
        status = json.loads(status_file.read_text())
        assert status["state"] in {"connection_error", "client_error"}
        assert status["event"] in {"eof", "run_error", "connect_error"}


def test_actual_client_explicit_stop_does_not_restart_or_fabricate_result():
    """Stopping a live real client must be terminal and result-neutral."""
    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    listener.listen(1)
    port = listener.getsockname()[1]
    server_error = []
    session_ready = threading.Event()

    def serve():
        try:
            connection, _ = listener.accept()
            with connection, connection.makefile("rwb") as stream:
                assert stream.readline().decode().startswith("LOGIN test ")
                stream.write(b"LOGIN: test OK\n")
                stream.flush()
                assert stream.readline().decode().startswith("%%GAME offline-stop")
                stream.write(
                    b"Game_ID:offline-stop\nYour_Turn:+\nTotal_Time:600\nByoyomi:10\n"
                    b"END Game_Summary\nBEGIN Position\nPI\nEND Position\nSTART:offline-stop\n"
                )
                stream.flush()
                assert stream.readline().decode().startswith("AGREE:offline-stop")
                session_ready.set()
                # Keep the session open; the supervisor must stop the client.
                connection.settimeout(3.0)
                while connection.recv(1024):
                    pass
        except (OSError, AssertionError) as error:  # pragma: no cover
            # Stopping the client closes the isolated TCP session. macOS may
            # report either a bad descriptor, a broken pipe, or ECONNRESET.
            if not isinstance(error, OSError) or error.errno not in {9, 53, 54}:
                server_error.append(error)
        finally:
            listener.close()

    server = threading.Thread(target=serve)
    server.start()
    with tempfile.TemporaryDirectory(prefix="sekirei-client-stop-") as directory:
        root = Path(directory)
        state_dir = root / "state"
        stop_file = root / "stop"
        status_file = root / "client-status.json"
        record_dir = root / "records"
        command = [
            str(ROOT / "target/debug/sekirei-csa"),
            "--server", "127.0.0.1", "--port", str(port),
            "--user", "test", "--password", "secret",
            "--eval", "material", "--depth", "1", "--hash", "1",
            "--game", "offline-stop", "--record-dir", str(record_dir),
            "--status-file", str(status_file),
        ]
        result_holder = {}

        def supervise():
            result_holder["result"] = supervisor.run(
                command, state_dir, stop_file, max_restarts=1, restart_window=60.0,
                restart_delay=0.0, poll_seconds=0.01, client_status_file=status_file,
                client_status_max_age=2.0, max_log_bytes=4096,
            )

        worker = threading.Thread(target=supervise)
        worker.start()
        events_path = state_dir / "events.jsonl"
        deadline = time.time() + 3.0
        while time.time() < deadline:
            if events_path.exists() and '"kind": "running"' in events_path.read_text():
                break
            time.sleep(0.01)
        else:
            raise AssertionError("real client did not reach running state")
        assert session_ready.wait(timeout=3.0), "real client did not finish the CSA handshake"
        stop_file.write_text("stop\n")
        worker.join(timeout=5.0)
        server.join(timeout=5.0)
        assert not worker.is_alive()
        assert not server.is_alive()
        assert not server_error, server_error
        assert result_holder["result"] == 0
        events = [json.loads(line) for line in events_path.read_text().splitlines()]
        assert supervisor_events.validate(events) == []
        assert [event["kind"] for event in events].count("started") == 1
        assert any(event["kind"] == "stop_requested" for event in events)
        assert events[-1]["exit_kind"] == "explicit_stop"
        assert all(event["kind"] != "restart_scheduled" for event in events)
        assert not any("#WIN" in path.read_text() or "#LOSE" in path.read_text() for path in record_dir.glob("*.csa"))


def test_actual_client_record_failure_is_not_a_loss_or_restart():
    """A real record initialization failure must abort before search/move."""
    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    listener.listen(1)
    port = listener.getsockname()[1]
    server_error = []

    def serve():
        try:
            connection, _ = listener.accept()
            with connection, connection.makefile("rwb") as stream:
                assert stream.readline().decode().startswith("LOGIN test ")
                stream.write(b"LOGIN: test OK\n")
                stream.flush()
                assert stream.readline().decode().startswith("%%GAME offline-record-failure")
                stream.write(
                    b"Game_ID:offline-record-failure\nYour_Turn:+\nTotal_Time:1\nByoyomi:1\n"
                    b"END Game_Summary\nBEGIN Position\nPI\nEND Position\nSTART:offline-record-failure\n"
                )
                stream.flush()
                assert stream.readline().decode().startswith("AGREE:offline-record-failure")
                connection.settimeout(3.0)
                assert connection.recv(1024) == b""
        except Exception as error:  # pragma: no cover - reported by the parent assertion
            server_error.append(error)
        finally:
            listener.close()

    server = threading.Thread(target=serve)
    server.start()
    with tempfile.TemporaryDirectory(prefix="sekirei-client-record-failure-") as directory:
        root = Path(directory)
        state_dir = root / "state"
        status_file = root / "client-status.json"
        record_path = root / "records-not-a-directory"
        record_path.write_text("not a directory\n", encoding="utf-8")
        command = [
            str(ROOT / "target/debug/sekirei-csa"),
            "--server", "127.0.0.1", "--port", str(port),
            "--user", "test", "--password", "secret",
            "--eval", "material", "--depth", "1", "--hash", "1",
            "--game", "offline-record-failure", "--record-dir", str(record_path),
            "--status-file", str(status_file), "--loop",
        ]
        result = supervisor.run(
            command, state_dir, root / "stop", max_restarts=1, restart_window=60.0,
            restart_delay=0.0, poll_seconds=0.01, client_status_file=status_file,
            client_status_max_age=2.0, max_log_bytes=4096,
        )
        server.join(timeout=5.0)
        assert not server.is_alive()
        assert not server_error, server_error
        assert result == 1
        events = [json.loads(line) for line in (state_dir / "events.jsonl").read_text().splitlines()]
        assert supervisor_events.validate(events) == []
        assert [event["kind"] for event in events].count("started") == 1
        assert all(event["kind"] != "restart_scheduled" for event in events)
        assert events[-1]["kind"] == "terminal_error", events
        status = json.loads(status_file.read_text())
        assert status["state"] == "client_error"
        assert status["event"] == "record_initialization_failed"


if __name__ == "__main__":
    test_actual_client_completes_one_offline_game()
    test_actual_client_connection_loss_is_bounded_by_supervisor()
    test_actual_client_explicit_stop_does_not_restart_or_fabricate_result()
    test_actual_client_record_failure_is_not_a_loss_or_restart()
    print("PASS")
