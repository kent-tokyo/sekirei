#!/usr/bin/env python3
"""Apply a predeclared load filter before interpreting component A/A captures."""
from __future__ import annotations

import argparse
import json
from pathlib import Path

from aggregate_component_aa import aggregate, evaluate_aa_window


def capture_load1(path: Path) -> float:
    document = json.loads((path / "provenance.json").read_text(encoding="utf-8"))
    value = document.get("replay_start_load1", document.get("capture_start_load1"))
    if not isinstance(value, (int, float)) or value < 0:
        raise ValueError(f"{path}: missing capture-start load")
    return float(value)


def capture_power_source(path: Path) -> str:
    document = json.loads((path / "provenance.json").read_text(encoding="utf-8"))
    value = document.get(
        "replay_start_power_source", document.get("capture_start_power_source")
    )
    if value not in {"ac", "battery", "unknown"}:
        raise ValueError(f"{path}: missing capture-start power source")
    return value


def filter_pairs(
    paths: list[Path], max_load1: float, require_ac: bool = False
) -> tuple[list[Path], list[dict]]:
    if len(paths) < 2 or len(paths) % 2:
        raise ValueError("capture count must be an even number >= 2")
    selected, excluded = [], []
    for left, right in zip(paths[::2], paths[1::2]):
        loads = [capture_load1(left), capture_load1(right)]
        power_sources = [capture_power_source(left), capture_power_source(right)]
        if max(loads) <= max_load1 and (not require_ac or all(source == "ac" for source in power_sources)):
            selected.extend((left, right))
        else:
            excluded.append({
                "left": str(left),
                "right": str(right),
                "load1": loads,
                "power_sources": power_sources,
            })
    return selected, excluded


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--max-load1", type=float, required=True)
    parser.add_argument(
        "--require-ac",
        action="store_true",
        help="Select only pairs captured on AC power",
    )
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("captures", nargs="+", type=Path)
    args = parser.parse_args()
    if args.max_load1 <= 0:
        parser.error("--max-load1 must be positive")
    try:
        selected, excluded = filter_pairs(args.captures, args.max_load1, args.require_ac)
        result = aggregate(selected) if len(selected) >= 2 else None
    except (OSError, ValueError, json.JSONDecodeError) as error:
        parser.error(str(error))
    status = evaluate_aa_window(result) if result and result["pair_count"] >= 2 else "INCONCLUSIVE"
    document = {
        "schema": "sekirei.component-aa-load-analysis.v1",
        "diagnostic_only": True,
        "max_load1": args.max_load1,
        "require_ac": args.require_ac,
        "selected_capture_count": len(selected),
        "excluded_pairs": excluded,
        "result": result,
        "status": status,
    }
    args.output.write_text(json.dumps(document, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {status}")
    return 0 if status == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
