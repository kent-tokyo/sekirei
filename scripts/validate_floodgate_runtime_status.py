#!/usr/bin/env python3
"""Validate a CSA runtime status snapshot used by an external monitor."""
from __future__ import annotations

import argparse
import json
import os
import sys
import time
from pathlib import Path


SCHEMA = "sekirei.csa-runtime-status.v1"
STATES = {
    "connecting",
    "authenticated",
    "waiting_for_game",
    "in_game",
    "receiving",
    "game_finished",
    "connection_error",
    "client_error",
}


def validate(document: object, expected_pid: int | None = None,
             max_age_ms: int | None = None, now_ms: int | None = None) -> list[str]:
    errors: list[str] = []
    if not isinstance(document, dict):
        return ["snapshot must be an object"]
    if document.get("schema") != SCHEMA:
        errors.append("schema mismatch")
    if document.get("state") not in STATES:
        errors.append("unknown state")
    pid = document.get("pid")
    if not isinstance(pid, int) or isinstance(pid, bool) or pid <= 0:
        errors.append("pid must be a positive integer")
    elif expected_pid is not None and pid != expected_pid:
        errors.append("pid does not match the expected process")
    timestamp = document.get("timestamp_ms")
    if not isinstance(timestamp, int) or isinstance(timestamp, bool) or timestamp <= 0:
        errors.append("timestamp_ms must be a positive integer")
    elif max_age_ms is not None:
        current = time.time_ns() // 1_000_000 if now_ms is None else now_ms
        if timestamp > current + 5_000:
            errors.append("timestamp_ms is in the future")
        elif current - timestamp > max_age_ms:
            errors.append("snapshot is stale")
    for field in ("request_game_id", "active_game_id", "session_id"):
        value = document.get(field)
        if value is not None and (not isinstance(value, str) or not value):
            errors.append(f"{field} must be a non-empty string or null")
    for field in ("last_server_received_ms", "last_our_move_sent_ms", "last_opponent_move_ms"):
        value = document.get(field)
        if value is not None and (not isinstance(value, int) or isinstance(value, bool) or value <= 0):
            errors.append(f"{field} must be a positive integer or null")
    if document.get("state") not in {"connecting", "connection_error", "client_error"}:
        if not document.get("session_id"):
            errors.append("active connection state requires session_id")
    if document.get("state") in {"in_game", "receiving"}:
        if not document.get("active_game_id"):
            errors.append("game state requires active_game_id")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("snapshot", type=Path)
    parser.add_argument("--pid", type=int, default=None)
    parser.add_argument("--max-age-ms", type=int, default=None)
    args = parser.parse_args()
    if args.max_age_ms is not None and args.max_age_ms < 0:
        parser.error("--max-age-ms must be non-negative")
    try:
        document = json.loads(args.snapshot.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        print(f"invalid runtime status: {error}", file=sys.stderr)
        return 1
    errors = validate(document, expected_pid=args.pid, max_age_ms=args.max_age_ms)
    if errors:
        print("invalid runtime status: " + "; ".join(errors), file=sys.stderr)
        return 1
    print("runtime status valid")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
