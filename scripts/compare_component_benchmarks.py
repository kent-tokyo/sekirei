#!/usr/bin/env python3
"""Compare two validated component benchmark captures.

The ratio is baseline_ns / candidate_ns, so values above 1.0 mean that the
candidate is faster. This is a diagnostic summary, not a strength claim.
"""

import argparse
import json
import math
from collections import defaultdict
from pathlib import Path

from run_component_benchmark import EXPECTED_CASES


def load_capture(path: Path) -> dict[tuple[str, str], float]:
    summary_path = path / "validated_summary.json"
    data = json.loads(summary_path.read_text(encoding="utf-8"))
    parsed = {}
    for key, value in data.items():
        operation, separator, library = key.partition("/")
        if not separator or not isinstance(value, dict):
            raise ValueError(f"invalid benchmark key: {key!r}")
        p50 = value.get("p50_ns")
        p95 = value.get("p95_ns")
        units = value.get("units")
        if not all(isinstance(item, (int, float)) and math.isfinite(item)
                   for item in (p50, p95)) or p50 <= 0 or p95 <= 0:
            raise ValueError(f"invalid timing for {key!r}")
        if not isinstance(units, int) or units <= 0:
            raise ValueError(f"invalid units for {key!r}")
        parsed[(operation, library)] = p50
    if set(parsed) != EXPECTED_CASES:
        missing = sorted(EXPECTED_CASES - set(parsed))
        extra = sorted(set(parsed) - EXPECTED_CASES)
        raise ValueError(f"case set mismatch: missing={missing}, extra={extra}")
    return parsed


def geometric_mean(values):
    values = list(values)
    if not values or any(value <= 0 for value in values):
        raise ValueError("geometric mean requires positive values")
    return math.exp(sum(math.log(value) for value in values) / len(values))


def compare(baseline: dict, candidate: dict) -> dict:
    ratios = {key: baseline[key] / candidate[key] for key in EXPECTED_CASES}
    grouped = defaultdict(list)
    for (operation, _library), ratio in ratios.items():
        grouped[operation].append(ratio)
    return {
        "case_count": len(ratios),
        "ratio_definition": "baseline_p50_ns / candidate_p50_ns",
        "ratios": {
            f"{operation}/{library}": ratios[(operation, library)]
            for operation, library in sorted(ratios)
        },
        "operation_geomean": {
            operation: geometric_mean(values)
            for operation, values in sorted(grouped.items())
        },
        "overall_geomean": geometric_mean(ratios.values()),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--json", action="store_true", dest="as_json")
    args = parser.parse_args()
    try:
        result = compare(load_capture(args.baseline), load_capture(args.candidate))
    except (OSError, ValueError, json.JSONDecodeError) as error:
        parser.error(str(error))
    if args.as_json:
        print(json.dumps(result, indent=2, sort_keys=True))
    else:
        print(f"cases: {result['case_count']}")
        print("ratio: baseline/candidate p50 (greater than 1 = candidate faster)")
        for operation, ratio in result["operation_geomean"].items():
            print(f"{operation}: {ratio:.4f}x")
        print(f"overall: {result['overall_geomean']:.4f}x")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
