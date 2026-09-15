#!/usr/bin/env python3
"""Tests for the offline supervisor event-stream validator."""
from __future__ import annotations

import unittest

from validate_floodgate_supervisor_events import validate


def event_stream(*kinds: str) -> list[dict[str, object]]:
    events: list[dict[str, object]] = []
    attempt = 0
    restart_count = 0
    for kind in kinds:
        if kind == "started":
            attempt += 1
            events.append({"kind": kind, "attempt": attempt})
        elif kind == "running":
            events.append({"kind": kind, "attempt": attempt})
        elif kind == "exited":
            exit_kind = "normal_exit" if kinds.count("started") == 1 else "exit_7"
            events.append({"kind": kind, "attempt": attempt, "child_exit_code": 0,
                           "exit_kind": exit_kind})
        elif kind == "restart_scheduled":
            restart_count += 1
            events.append({"kind": kind, "restart_count": restart_count})
        else:
            events.append({"kind": kind})
    return events


class SupervisorEventTests(unittest.TestCase):
    def test_normal_exit_stream_is_valid(self) -> None:
        self.assertEqual(validate(event_stream("started", "running", "exited")), [])

    def test_bounded_restart_stream_is_valid(self) -> None:
        events = event_stream("started", "running", "exited", "restart_scheduled",
                              "started", "running", "exited", "circuit_breaker")
        self.assertEqual(validate(events), [])

    def test_restart_after_terminal_is_rejected(self) -> None:
        events = [
            {"kind": "started", "attempt": 1},
            {"kind": "running", "attempt": 1},
            {"kind": "exited", "attempt": 1, "child_exit_code": 0,
             "exit_kind": "normal_exit"},
            {"kind": "started", "attempt": 2},
        ]
        self.assertTrue(any("after terminal" in error for error in validate(events)))

    def test_missing_exit_code_is_rejected(self) -> None:
        events = event_stream("started", "running", "exited")
        del events[-1]["child_exit_code"]
        self.assertTrue(any("child_exit_code" in error for error in validate(events)))


if __name__ == "__main__":
    unittest.main()
