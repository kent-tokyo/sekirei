#!/usr/bin/env python3
"""Write a reproducible, low-cost measurement contract manifest.

This records inputs and environment only.  It deliberately does not run a
strength gate or claim an A/A noise floor; those require the heavyweight
repeats harness and are marked NOT_RUN until executed with a frozen binary.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import platform
import subprocess
from datetime import datetime, timezone
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def run(*args: str) -> str:
    try:
        return subprocess.check_output(args, cwd=ROOT, text=True, stderr=subprocess.STDOUT).strip()
    except (OSError, subprocess.CalledProcessError) as exc:
        return f"unavailable: {exc}"


def sha256(path: Path | None) -> str | None:
    if path is None or not path.is_file():
        return None
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--weights", type=Path)
    parser.add_argument("--spec-top-n", type=int, default=0)
    parser.add_argument("--option", action="append", default=[], metavar="NAME=VALUE")
    args = parser.parse_args()

    options = {item.split("=", 1)[0]: item.split("=", 1)[1] for item in args.option if "=" in item}
    binary = args.binary.resolve() if args.binary else None
    weights = args.weights.resolve() if args.weights else None
    manifest = {
        "schema": "sekirei.measurement-contract.v1",
        "measurement_class": "contract_only",
        "recorded_at": datetime.now(timezone.utc).isoformat(),
        "repository": {
            "sha": run("git", "rev-parse", "HEAD"),
            "branch": run("git", "branch", "--show-current"),
            "worktree_status": run("git", "status", "--short"),
        },
        "toolchain": {
            "rustc": run("rustc", "-Vv"),
            "cargo": run("cargo", "-V"),
        },
        "artifact": {
            "binary": str(binary) if binary else None,
            "binary_sha256": sha256(binary),
            "weights": str(weights) if weights else None,
            "weights_sha256": sha256(weights),
        },
        "hardware": {
            "system": platform.platform(),
            "machine": platform.machine(),
            "processor": platform.processor(),
            "python": platform.python_version(),
            "uname": run("uname", "-a"),
        },
        "options": options,
        "search_contract": {
            "spec_top_n": args.spec_top_n,
            "deterministic_control": args.spec_top_n == 0,
            "aa_noise_floor": {
                "status": "NOT_RUN",
                "protocol": "repeats",
                "reason": "Run only after the binary, options, and hardware are frozen.",
            },
        },
        "gate3_contract": {
            "status": "FROZEN_NOT_RUN",
            "comparison": "official NNUE v1 A-flat vs material baseline",
            "heavy_measurement": "not started by this script",
        },
    }
    output = args.output if args.output.is_absolute() else ROOT / args.output
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n")
    print(f"wrote {output}")


if __name__ == "__main__":
    main()
