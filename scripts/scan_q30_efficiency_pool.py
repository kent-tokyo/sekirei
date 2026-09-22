#!/usr/bin/env python3
"""Build Q30's score-free CSA pool for the NNUE efficiency frontier."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

import scan_q29_coverage_pool as q29


SCHEMA = "sekirei.q30-efficiency-pool-scan.v1"


def run(args: argparse.Namespace) -> dict[str, Any]:
    document = q29.run(args)
    document["schema"] = SCHEMA
    document["contract"]["exclusive_factor"] = "NNUE capacity efficiency frontier only"
    return document


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--csa-source-dir", type=Path, required=True)
    parser.add_argument("--history-replay", type=Path, required=True)
    parser.add_argument("--exclude-runs-root", type=Path, required=True)
    parser.add_argument("--exclude-reserve", type=Path, action="append", default=[])
    parser.add_argument("--source-scan-cap", type=int, default=512)
    parser.add_argument("--seed", type=int, default=3001)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        parser.error("output already exists; use a new scan path")
    try:
        document = run(args)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
