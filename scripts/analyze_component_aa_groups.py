#!/usr/bin/env python3
"""Split component A/A captures into predeclared operation families."""
from __future__ import annotations

import argparse
import json
import math
from collections import defaultdict
from pathlib import Path

from aggregate_component_aa import evaluate_aa_window, geometric_mean, geometric_mean_ci95
from compare_component_benchmarks import load_capture


def family(operation: str, library: str) -> str:
    key = f"{operation}/{library}"
    if operation.startswith("init_") or operation == "harness_dispatch_floor":
        return "initialization"
    if "nnue" in key or "evaluate_with_weights" in key:
        return "nnue"
    if "do_undo" in key or "roundtrip" in operation:
        return "board_update"
    if operation in {"encode_raw_list", "decode_packed_list"} or "packed" in library or "narrow" in library:
        return "move_representation"
    return "move_generation"


def analyze(paths: list[Path]) -> dict:
    if len(paths) < 2 or len(paths) % 2:
        raise ValueError("capture count must be an even number >= 2")
    captures = [load_capture(path) for path in paths]
    keys = set(captures[0])
    if any(set(capture) != keys for capture in captures[1:]):
        raise ValueError("capture case sets differ")
    grouped_pairs: dict[str, list[float]] = defaultdict(list)
    group_cases: dict[str, list[str]] = defaultdict(list)
    for operation, library in sorted(keys):
        group_cases[family(operation, library)].append(f"{operation}/{library}")
    for left, right in zip(captures[::2], captures[1::2]):
        per_group: dict[str, list[float]] = defaultdict(list)
        for operation, library in keys:
            per_group[family(operation, library)].append(left[(operation, library)] / right[(operation, library)])
        for name, ratios in per_group.items():
            grouped_pairs[name].append(geometric_mean(ratios))
    groups = {}
    for name, ratios in sorted(grouped_pairs.items()):
        low, high = geometric_mean_ci95(ratios)
        groups[name] = {
            "case_count": len(group_cases[name]),
            "cases": group_cases[name],
            "pair_geomeans": ratios,
            "overall_geomean": geometric_mean(ratios),
            "ci95": {"low": low, "high": high},
            "status": evaluate_aa_window({"pair_geomean_ci95": {"low": low, "high": high}}),
        }
    return {
        "schema": "sekirei.component-aa-group-analysis.v1",
        "diagnostic_only": True,
        "pair_count": len(paths) // 2,
        "groups": groups,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("captures", nargs="+", type=Path)
    args = parser.parse_args()
    try:
        document = analyze(args.captures)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.write_text(json.dumps(document, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
