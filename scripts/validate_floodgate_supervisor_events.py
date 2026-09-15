#!/usr/bin/env python3
"""Validate the bounded supervisor event stream without contacting Floodgate."""
from __future__ import annotations

import argparse
import json
from pathlib import Path


TERMINAL_KINDS = {
    "normal_exit",
    "explicit_stop_before_start",
    "circuit_breaker",
    "terminal_error",
    "start_failed",
}


def validate(events: object) -> list[str]:
    if not isinstance(events, list) or not events:
        return ["events must be a non-empty list"]

    errors: list[str] = []
    seen_terminal = False
    running_attempts: set[int] = set()
    restart_count = 0
    for index, event in enumerate(events):
        if not isinstance(event, dict):
            errors.append(f"event {index}: must be an object")
            continue
        kind = event.get("kind")
        if not isinstance(kind, str) or not kind:
            errors.append(f"event {index}: missing kind")
            continue
        if seen_terminal:
            errors.append(f"event {index}: event after terminal {kind}")
        if kind == "started":
            attempt = event.get("attempt")
            if not isinstance(attempt, int) or isinstance(attempt, bool) or attempt <= 0:
                errors.append(f"event {index}: invalid started attempt")
            elif attempt in running_attempts:
                errors.append(f"event {index}: duplicate started attempt")
            else:
                running_attempts.add(attempt)
        elif kind == "running":
            attempt = event.get("attempt")
            if attempt not in running_attempts:
                errors.append(f"event {index}: running without started attempt")
        elif kind == "exited":
            attempt = event.get("attempt")
            if attempt not in running_attempts:
                errors.append(f"event {index}: exited without started attempt")
            else:
                running_attempts.remove(attempt)
            if "child_exit_code" not in event:
                errors.append(f"event {index}: exited missing child_exit_code")
            if event.get("exit_kind") in {"normal_exit", "explicit_stop"}:
                seen_terminal = True
        elif kind == "restart_scheduled":
            restart_count += 1
            if event.get("restart_count") != restart_count:
                errors.append(f"event {index}: restart_count is not monotonic")
        elif kind in {"explicit_stop", "client_status_invalid", "client_status_observed",
                      "client_status_changed", "client_session_changed", "stop_requested"}:
            pass
        elif kind in TERMINAL_KINDS:
            seen_terminal = True
        else:
            errors.append(f"event {index}: unknown kind {kind!r}")

    if running_attempts:
        errors.append("events end with a running attempt")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("events", type=Path)
    args = parser.parse_args()
    try:
        lines = [line for line in args.events.read_text(encoding="utf-8").splitlines() if line.strip()]
        document = [json.loads(line) for line in lines]
    except (OSError, json.JSONDecodeError) as error:
        print(f"invalid supervisor events: {error}")
        return 1
    errors = validate(document)
    if errors:
        print("invalid supervisor events: " + "; ".join(errors))
        return 1
    print("supervisor events valid")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
