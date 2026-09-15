import importlib.util
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "runtime_status", ROOT / "scripts/validate_floodgate_runtime_status.py"
)
runtime_status = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(runtime_status)


def valid_snapshot():
    now = 1_700_000_000_000
    return {
        "schema": runtime_status.SCHEMA,
        "state": "in_game",
        "event": "opponent_move",
        "timestamp_ms": now,
        "pid": os.getpid(),
        "request_game_id": "floodgate-300-10F",
        "active_game_id": "game-42",
        "session_id": "123-456",
        "last_server_received_ms": now - 10,
        "last_our_move_sent_ms": now - 20,
        "last_opponent_move_ms": now - 10,
    }


def test_valid_snapshot_and_generation_fields():
    document = valid_snapshot()
    assert runtime_status.validate(document, expected_pid=os.getpid(), now_ms=1_700_000_001_000,
                                   max_age_ms=2_000) == []


def test_stale_or_wrong_generation_is_rejected():
    document = valid_snapshot()
    assert "pid does not match the expected process" in runtime_status.validate(document, expected_pid=1)
    assert "snapshot is stale" in runtime_status.validate(document, now_ms=1_700_000_010_000,
                                                            max_age_ms=2_000)


def test_waiting_state_can_have_no_active_game():
    document = valid_snapshot()
    document.update({"state": "waiting_for_game", "active_game_id": None, "event": None})
    assert runtime_status.validate(document) == []


def test_cli_validates_a_snapshot():
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "status.json"
        path.write_text(json.dumps(valid_snapshot()), encoding="utf-8")
        result = subprocess.run(
            [sys.executable, str(ROOT / "scripts/validate_floodgate_runtime_status.py"),
             str(path), "--pid", str(os.getpid())],
            capture_output=True, text=True, check=False,
        )
        assert result.returncode == 0, result.stderr


if __name__ == "__main__":
    test_valid_snapshot_and_generation_fields()
    test_stale_or_wrong_generation_is_rejected()
    test_waiting_state_can_have_no_active_game()
    test_cli_validates_a_snapshot()
    print("PASS")
