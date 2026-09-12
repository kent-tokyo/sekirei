#!/usr/bin/env python3
"""Aggregate adjacent same-binary component captures as A/A pairs."""

import argparse
import json
import math
import statistics
from pathlib import Path

from compare_component_benchmarks import load_capture


def geometric_mean(values):
    values = list(values)
    if not values or any(value <= 0 for value in values):
        raise ValueError("geometric mean requires positive values")
    return math.exp(sum(math.log(value) for value in values) / len(values))


def geometric_mean_ci95(values):
    """Return a small-sample t interval for a geometric mean of ratios."""
    values = list(values)
    if len(values) < 2 or any(value <= 0 for value in values):
        raise ValueError("at least two positive ratios are required")
    logs = [math.log(value) for value in values]
    mean = statistics.mean(logs)
    half_width = 2.776 * statistics.stdev(logs) / math.sqrt(len(logs))
    return math.exp(mean - half_width), math.exp(mean + half_width)


def aggregate(paths):
    paths = list(paths)
    if len(paths) < 2 or len(paths) % 2:
        raise ValueError("A/A capture count must be an even number >= 2")
    captures = [load_capture(path) for path in paths]
    keys = sorted(captures[0])
    if any(sorted(capture) != keys for capture in captures[1:]):
        raise ValueError("capture case sets differ")

    pair_rows = []
    by_case = {key: [] for key in keys}
    for index in range(0, len(captures), 2):
        ratios = {
            key: captures[index][key] / captures[index + 1][key]
            for key in keys
        }
        for key, ratio in ratios.items():
            by_case[key].append(ratio)
        pair_rows.append({
            "left": str(paths[index]),
            "right": str(paths[index + 1]),
            "geomean": geometric_mean(ratios.values()),
            "min": min(ratios.values()),
            "max": max(ratios.values()),
        })

    case_rows = {
        f"{operation}/{library}": {
            "median": statistics.median(values),
            "min": min(values),
            "max": max(values),
        }
        for (operation, library), values in sorted(by_case.items())
    }
    pair_geomeans = [row["geomean"] for row in pair_rows]
    ci_low, ci_high = geometric_mean_ci95(pair_geomeans)
    return {
        "pair_count": len(pair_rows),
        "case_count": len(keys),
        "pair_rows": pair_rows,
        "case_rows": case_rows,
        "overall_geomean": geometric_mean(
            ratio for values in by_case.values() for ratio in values
        ),
        "pair_geomean_ci95": {"low": ci_low, "high": ci_high},
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("captures", nargs="+", type=Path)
    parser.add_argument("--json", action="store_true", dest="as_json")
    args = parser.parse_args()
    try:
        result = aggregate(args.captures)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        parser.error(str(error))
    if args.as_json:
        print(json.dumps(result, indent=2, sort_keys=True))
    else:
        print(f"pairs: {result['pair_count']}; cases: {result['case_count']}")
        print(f"overall geomean: {result['overall_geomean']:.4f}x")
        ci = result["pair_geomean_ci95"]
        print(f"pair geomean 95% CI: {ci['low']:.4f}..{ci['high']:.4f}x")
        for row in result["pair_rows"]:
            print(
                f"{row['left']} vs {row['right']}: "
                f"{row['geomean']:.4f}x "
                f"(range {row['min']:.4f}..{row['max']:.4f})"
            )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
