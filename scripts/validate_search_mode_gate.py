#!/usr/bin/env python3
"""Validate the fixed protocol for the local SearchMode comparison gate."""

from __future__ import annotations

import argparse
import json
from pathlib import Path


REQUIRED = {
    "schema": "sekirei.search-mode-strength-gate-plan.v1",
    "purpose": "isolate SearchMode implementation effects with identical binary and weights",
}


def validate(plan: dict) -> list[str]:
    errors: list[str] = []
    for key, expected in REQUIRED.items():
        if plan.get(key) != expected:
            errors.append(f"{key}: expected {expected!r}")
    if plan.get("status") not in {"planned", "running", "complete", "inconclusive"}:
        errors.append("status: unsupported")
    if plan.get("positions") != 99 or plan.get("games_per_position") != 2:
        errors.append("protocol: must cover 99 positions with two games each")
    if plan.get("byoyomi_ms") != 100 or plan.get("max_games") != 198:
        errors.append("protocol: must use 100 ms byoyomi and 198 games")
    for side in ("engine1", "engine2"):
        options = plan.get(side)
        if not isinstance(options, dict):
            errors.append(f"{side}: missing options")
            continue
        for key, value in {"Threads": "1", "SpecTopN": "0", "UseBook": "false"}.items():
            if options.get(key) != value:
                errors.append(f"{side}.{key}: expected {value!r}")
    if plan.get("engine1", {}).get("SearchMode") != "Speculative":
        errors.append("engine1.SearchMode: expected Speculative")
    if plan.get("engine2", {}).get("SearchMode") != "LazySMP":
        errors.append("engine2.SearchMode: expected LazySMP")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("plan", type=Path)
    args = parser.parse_args()
    errors = validate(json.loads(args.plan.read_text(encoding="utf-8")))
    if errors:
        for error in errors:
            print(f"ERROR: {error}")
        return 1
    print("search-mode gate plan OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
