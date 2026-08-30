#!/usr/bin/env python3
"""Compare NPS/depth/think-time between a serial and a parallel run of the
B-vs-C YBW gate, using the info-line logs captured by scripts/tee_engine_{b,c}.sh
(throwaway load-test instrumentation only -- see those scripts for why this
never runs against the production gate).

Usage: python3 scripts/analyze_loadtest.py <label> <logfile> [<logfile> ...]
"""
import re
import statistics
import sys

INFO_RE = re.compile(
    r"^info depth (\d+) score cp (-?\d+) nodes (\d+) nps (\d+) time (\d+)"
)


def parse(paths):
    depths, npss, times = [], [], []
    for path in paths:
        try:
            with open(path) as f:
                for line in f:
                    m = INFO_RE.match(line)
                    if m:
                        depths.append(int(m.group(1)))
                        npss.append(int(m.group(4)))
                        times.append(int(m.group(5)))
        except FileNotFoundError:
            pass
    return depths, npss, times


def pct(values, p):
    if not values:
        return 0.0
    s = sorted(values)
    idx = min(len(s) - 1, int(round(p / 100 * (len(s) - 1))))
    return s[idx]


def report(label, paths):
    depths, npss, times = parse(paths)
    if not depths:
        print(f"{label}: no info lines found in {paths}")
        return
    print(
        f"{label}: n={len(depths)}  "
        f"depth mean={statistics.mean(depths):.2f} median={statistics.median(depths):.1f}  "
        f"nps mean={statistics.mean(npss):.0f} median={statistics.median(npss):.0f}  "
        f"time(ms) mean={statistics.mean(times):.0f} p95={pct(times, 95):.0f} max={max(times)}"
    )


def main():
    if len(sys.argv) < 3:
        print("usage: analyze_loadtest.py <label> <logfile> [more...]", file=sys.stderr)
        sys.exit(1)
    report(sys.argv[1], sys.argv[2:])


if __name__ == "__main__":
    main()
