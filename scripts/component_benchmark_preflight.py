#!/usr/bin/env python3
"""Check whether the host is eligible for a controlled component A/A capture."""
from __future__ import annotations

import argparse
import json
import platform
from datetime import datetime, timezone
from pathlib import Path

from run_component_benchmark import load1, power_source, thermal_status


def preflight(max_load1: float, require_ac: bool, observed_load1: float, observed_power: str,
              observed_thermal: dict[str, str]) -> dict:
    checks = {
        "load1": {"actual": observed_load1, "maximum": max_load1, "pass": observed_load1 <= max_load1},
        "power": {
            "actual": observed_power,
            "required": "ac" if require_ac else None,
            "pass": not require_ac or observed_power == "ac",
        },
        "thermal_warning": {
            "actual": observed_thermal["thermal_warning"],
            "pass": observed_thermal["thermal_warning"] == "none",
        },
        "performance_warning": {
            "actual": observed_thermal["performance_warning"],
            "pass": observed_thermal["performance_warning"] == "none",
        },
    }
    return {
        "schema": "sekirei.component-benchmark-preflight.v1",
        "checked_utc": datetime.now(timezone.utc).isoformat(),
        "platform": platform.platform(),
        "checks": checks,
        "verdict": "PASS" if all(check["pass"] for check in checks.values()) else "REFUSE",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--max-load1", type=float, default=2.0)
    parser.add_argument("--require-ac", action="store_true")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.max_load1 <= 0:
        parser.error("--max-load1 must be positive")
    document = preflight(args.max_load1, args.require_ac, load1(), power_source(), thermal_status())
    text = json.dumps(document, indent=2) + "\n"
    if args.output:
        args.output.write_text(text, encoding="utf-8")
    else:
        print(text, end="")
    return 0 if document["verdict"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
