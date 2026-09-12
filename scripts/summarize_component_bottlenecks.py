#!/usr/bin/env python3
"""Rank component timing mass and report a clearly-labelled Amdahl diagnostic."""

import argparse
import json
import math
from pathlib import Path


def summarize(summary: dict, hypothetical_speedup: float = 2.0, top: int = 3) -> dict:
    if not math.isfinite(hypothetical_speedup) or hypothetical_speedup <= 1:
        raise ValueError("hypothetical speedup must be greater than 1")
    rows = []
    for key, value in summary.items():
        operation, separator, library = key.partition("/")
        if not separator or not isinstance(value, dict) or not library.startswith("sekirei"):
            continue
        p50 = value.get("p50_ns")
        if not isinstance(p50, (int, float)) or not math.isfinite(p50) or p50 <= 0:
            raise ValueError(f"invalid p50 for {key}")
        rows.append({"operation": operation, "library": library, "p50_ns": float(p50)})
    if not rows:
        raise ValueError("summary contains no Sekirei rows")
    total = sum(row["p50_ns"] for row in rows)
    for row in rows:
        fraction = row["p50_ns"] / total
        row["fraction_of_component_sum"] = fraction
        row["amdahl_upper_bound"] = 1 / ((1 - fraction) + fraction / hypothetical_speedup)
    rows.sort(key=lambda row: (-row["p50_ns"], row["operation"], row["library"]))
    return {
        "schema": "sekirei.component-bottleneck-diagnostic.v1",
        "scope": "Sekirei component p50 sum; not an end-to-end workload",
        "hypothetical_speedup": hypothetical_speedup,
        "component_sum_ns": total,
        "top": rows[:top],
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("summary", type=Path)
    parser.add_argument("--hypothetical-speedup", type=float, default=2.0)
    parser.add_argument("--top", type=int, default=3)
    args = parser.parse_args()
    if args.top <= 0:
        parser.error("--top must be positive")
    try:
        result = summarize(json.loads(args.summary.read_text(encoding="utf-8")), args.hypothetical_speedup, args.top)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        parser.error(str(error))
    print(json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
