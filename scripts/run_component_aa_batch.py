#!/usr/bin/env python3
"""Run a same-binary component A/A batch with a fresh strict preflight per capture."""
from __future__ import annotations

import argparse
import json
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
PREFLIGHT = ROOT / "scripts/component_benchmark_preflight.py"
CAPTURE = ROOT / "scripts/run_component_benchmark.py"


def capture_name(index: int) -> str:
    return f"aa{index:02d}"


def strict_flags(max_load1: float) -> list[str]:
    return [
        "--max-load1", str(max_load1), "--require-ac", "--require-thermal-normal",
    ]


def build_capture_command(binary: Path, output: Path, preflight: Path, replay: Path | None,
                          max_load1: float) -> list[str]:
    source = ["--replay", str(replay)] if replay else ["--binary", str(binary)]
    return [sys.executable, str(CAPTURE), *source, "--output", str(output),
            "--preflight", str(preflight), *strict_flags(max_load1)]


def run_command(command: list[str]) -> None:
    subprocess.run(command, cwd=ROOT, check=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True,
                        help="New directory containing captures aa01..aaNN and their preflights")
    parser.add_argument("--pairs", type=int, default=5)
    parser.add_argument("--max-load1", type=float, default=2.0)
    args = parser.parse_args()
    if args.pairs < 2:
        parser.error("--pairs must be at least 2 for a 95% A/A interval")
    if args.max_load1 <= 0:
        parser.error("--max-load1 must be positive")
    if args.output.exists():
        parser.error(f"output already exists: {args.output}")
    binary = args.binary.resolve(strict=True)
    args.output.mkdir(parents=True)
    plan = {
        "schema": "sekirei.component-aa-batch-plan.v1",
        "created_utc": datetime.now(timezone.utc).isoformat(),
        "binary": str(binary),
        "pairs": args.pairs,
        "strict_flags": strict_flags(args.max_load1),
        "captures": [capture_name(index) for index in range(1, args.pairs * 2 + 1)],
    }
    (args.output / "plan.json").write_text(json.dumps(plan, indent=2) + "\n", encoding="utf-8")
    first_capture = args.output / capture_name(1)
    for index in range(1, args.pairs * 2 + 1):
        name = capture_name(index)
        preflight = args.output / f"preflight-{name}.json"
        run_command([sys.executable, str(PREFLIGHT), *strict_flags(args.max_load1), "--output", str(preflight)])
        run_command(build_capture_command(
            binary, args.output / name, preflight,
            None if index == 1 else first_capture, args.max_load1,
        ))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
