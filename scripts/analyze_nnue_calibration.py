#!/usr/bin/env python3
"""Compare NNUE checkpoint calibration on a fixed SFEN corpus.

This is a lightweight diagnostic, not a playing-strength measurement.  It
reports score spread, correlation, and robust candidate-vs-baseline deltas for
one or more checkpoints.  The engine-side probe is intentionally reused so
the analysis cannot accidentally implement a second evaluator.
"""

import argparse
import json
import math
import statistics
import subprocess
from pathlib import Path


def probe(binary: Path, weights: Path, sfens: list[str]) -> list[int]:
    # A calibration comparison is invalid if either checkpoint is constant or
    # changes after reload.  Enforce the same health gate used by direct probes
    # before interpreting any candidate-vs-baseline statistic.
    argv = [str(binary), str(weights), "--strict", "--json"]
    for sfen in sfens:
        argv.extend(("--sfen", sfen))
    result = subprocess.run(argv, check=True, capture_output=True, text=True)
    return [int(p["score_cp"]) for p in json.loads(result.stdout)["probes"]]


def correlation(xs: list[int], ys: list[int]) -> float:
    mx, my = statistics.mean(xs), statistics.mean(ys)
    numerator = sum((x - mx) * (y - my) for x, y in zip(xs, ys))
    x_norm = math.sqrt(sum((x - mx) ** 2 for x in xs))
    y_norm = math.sqrt(sum((y - my) ** 2 for y in ys))
    return numerator / (x_norm * y_norm) if x_norm and y_norm else 0.0


def summary(label: str, xs: list[int], baseline: list[int]) -> dict:
    deltas = [x - y for x, y in zip(xs, baseline)]
    absolute = sorted(abs(x) for x in deltas)
    return {
        "label": label,
        "mean_cp": statistics.mean(xs),
        "median_cp": statistics.median(xs),
        "std_cp": statistics.pstdev(xs),
        "range_cp": max(xs) - min(xs),
        "correlation_with_baseline": correlation(xs, baseline),
        "mean_delta_cp": statistics.mean(deltas),
        "mean_abs_delta_cp": statistics.mean(absolute),
        "p95_abs_delta_cp": absolute[int(0.95 * (len(absolute) - 1))],
        "max_abs_delta_cp": max(absolute),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/release/nnue_probe"))
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--candidate", action="append", nargs=2, metavar=("LABEL", "WEIGHTS"), required=True)
    parser.add_argument("--limit", type=int, default=100)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()

    if args.limit <= 0:
        parser.error("--limit must be positive")
    sfens = [line.strip() for line in args.corpus.read_text().splitlines() if line.strip() and not line.startswith("#")]
    sfens = sfens[: args.limit]
    if not sfens:
        parser.error("corpus contains no SFEN records")

    baseline = probe(args.binary, args.baseline, sfens)
    rows = [summary("baseline", baseline, baseline)]
    for label, path in args.candidate:
        rows.append(summary(label, probe(args.binary, Path(path), sfens), baseline))
    report = {"positions": len(sfens), "baseline": str(args.baseline), "candidates": rows}
    if args.json:
        print(json.dumps(report, ensure_ascii=False, indent=2))
    else:
        print("label mean median std range corr mean_delta mean_abs p95_abs max_abs")
        for row in rows:
            print(row["label"], *(f"{row[key]:.3f}" if isinstance(row[key], float) else row[key] for key in (
                "mean_cp", "median_cp", "std_cp", "range_cp", "correlation_with_baseline",
                "mean_delta_cp", "mean_abs_delta_cp", "p95_abs_delta_cp", "max_abs_delta_cp")))


if __name__ == "__main__":
    main()
