#!/usr/bin/env python3
"""Unit tests for the offline Denryu rehearsal evidence helpers."""
from __future__ import annotations

import importlib.util
import json
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "run_denryu_rehearsal", ROOT / "scripts/run_denryu_rehearsal.py"
)
module = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(module)


def test_canonical_hash_is_order_independent() -> None:
    assert module.canonical_sha256({"a": 1, "b": 2}) == module.canonical_sha256(
        {"b": 2, "a": 1}
    )


def test_journal_requires_two_monotonic_terminal_boundaries() -> None:
    phases = [
        {"pid": 101, "final_completed_attempts": 7},
        {"pid": 202, "final_completed_attempts": 14},
    ]
    rows = [
        {"pid": 101, "state": "connecting", "completed_attempts": 0},
        {
            "pid": 101,
            "state": "stopped",
            "completed_attempts": 7,
            "terminal_stop_reason": "max_games_reached",
        },
        {"pid": 202, "state": "connecting", "completed_attempts": 7},
        {
            "pid": 202,
            "state": "stopped",
            "completed_attempts": 14,
            "terminal_stop_reason": "max_games_reached",
        },
    ]
    with tempfile.TemporaryDirectory(prefix="sekirei-denryu-journal-") as directory:
        path = Path(directory) / "journal.jsonl"
        path.write_text("".join(json.dumps(row) + "\n" for row in rows), encoding="utf-8")
        assert module.validate_journal(path, phases, 14) == 4


def test_journal_rejects_decreasing_progress() -> None:
    phases = [
        {"pid": 101, "final_completed_attempts": 7},
        {"pid": 202, "final_completed_attempts": 14},
    ]
    rows = [
        {"pid": 101, "state": "connecting", "completed_attempts": 0},
        {
            "pid": 101,
            "state": "stopped",
            "completed_attempts": 7,
            "terminal_stop_reason": "max_games_reached",
        },
        {"pid": 202, "state": "connecting", "completed_attempts": 6},
        {
            "pid": 202,
            "state": "stopped",
            "completed_attempts": 14,
            "terminal_stop_reason": "max_games_reached",
        },
    ]
    with tempfile.TemporaryDirectory(prefix="sekirei-denryu-journal-") as directory:
        path = Path(directory) / "journal.jsonl"
        path.write_text("".join(json.dumps(row) + "\n" for row in rows), encoding="utf-8")
        try:
            module.validate_journal(path, phases, 14)
        except ValueError as error:
            assert "not monotonic" in str(error)
        else:
            raise AssertionError("decreasing progress was accepted")


if __name__ == "__main__":
    test_canonical_hash_is_order_independent()
    test_journal_requires_two_monotonic_terminal_boundaries()
    test_journal_rejects_decreasing_progress()
    print("PASS")
