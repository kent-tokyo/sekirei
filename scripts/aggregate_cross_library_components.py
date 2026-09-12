#!/usr/bin/env python3
"""Aggregate paired Sekirei/rsshogi component rows across captures."""

import argparse
import json
import math
import statistics
from pathlib import Path

from compare_component_benchmarks import load_capture

PAIRS = {
    "startpos/legal_generation": (("startpos", "sekirei_generate_vec"), ("startpos", "rsshogi_generate_move32")),
    "midgame/legal_generation": (("midgame", "sekirei_generate_vec"), ("midgame", "rsshogi_generate_move32")),
    "drop_only/legal_generation": (("drop_only", "sekirei_generate_vec"), ("drop_only", "rsshogi_generate_move32")),
}


def geometric_mean(values):
    values = list(values)
    if not values or any(value <= 0 for value in values):
        raise ValueError("geometric mean requires positive values")
    return math.exp(sum(math.log(value) for value in values) / len(values))


def ci95(values):
    values = list(values)
    if len(values) < 2:
        raise ValueError("at least two sessions are required")
    logs = [math.log(value) for value in values]
    half_width = 2.262 * statistics.stdev(logs) / math.sqrt(len(logs))
    mean = statistics.mean(logs)
    return math.exp(mean - half_width), math.exp(mean + half_width)


def aggregate(paths):
    paths = list(paths)
    if len(paths) != 10:
        raise ValueError("exactly ten captures are required")
    captures = [load_capture(path) for path in paths]
    ratios = {}
    session_rows = []
    for index, capture in enumerate(captures, 1):
        row = {}
        for name, (sekirei, rsshogi) in PAIRS.items():
            if sekirei not in capture or rsshogi not in capture:
                raise ValueError(f"capture {index} is missing {name}")
            row[name] = capture[rsshogi] / capture[sekirei]
            ratios.setdefault(name, []).append(row[name])
        session_rows.append({"session": index, "ratios": row})
    case_rows = {}
    for name, values in ratios.items():
        low, high = ci95(values)
        case_rows[name] = {
            "geomean": geometric_mean(values),
            "ci95": {"low": low, "high": high},
            "min": min(values),
            "max": max(values),
        }
    session_geomeans = [geometric_mean(row["ratios"].values()) for row in session_rows]
    low, high = ci95(session_geomeans)
    return {
        "schema": "sekirei.cross-library-component-summary.v1",
        "session_count": len(paths),
        "case_count": len(PAIRS),
        "direction": "rsshogi_divided_by_sekirei",
        "case_rows": case_rows,
        "session_geomeans": session_geomeans,
        "overall_geomean": geometric_mean(session_geomeans),
        "overall_ci95": {"low": low, "high": high},
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("captures", nargs=10, type=Path)
    parser.add_argument("--json", action="store_true", dest="as_json")
    args = parser.parse_args()
    try:
        result = aggregate(args.captures)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        parser.error(str(error))
    if args.as_json:
        print(json.dumps(result, indent=2, sort_keys=True))
    else:
        print(f"sessions: {result['session_count']}; cases: {result['case_count']}")
        print(f"overall rsshogi/Sekirei: {result['overall_geomean']:.4f}x")
        ci = result["overall_ci95"]
        print(f"overall 95% CI: {ci['low']:.4f}..{ci['high']:.4f}x")
        for name, row in result["case_rows"].items():
            print(f"{name}: {row['geomean']:.4f}x ({row['ci95']['low']:.4f}..{row['ci95']['high']:.4f})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
