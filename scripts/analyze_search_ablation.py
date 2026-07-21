#!/usr/bin/env python3
"""Paired arm-vs-arm delta analysis over search_ablation JSONL output.

The raw JSONL from `search_ablation fixed-depth`/`fixed-time` is the primary
artifact; this script only derives paired per-position comparisons from it
(never sums records naively across positions -- a single heavy position
would otherwise dominate the aggregate).

For each requested arm-pair comparison, at matching (position_id, depth or
time-mode, threads, profile), this computes each arm's per-repetition MEDIAN
value first (collapsing reps), then a per-position relative delta, then
aggregates those per-position deltas: median, mean, a seeded bootstrap 95%
CI, and improved/worsened/tied position counts, plus a category breakdown
(category = position_id's prefix before the trailing "_NNN").

Usage:
    python3 scripts/analyze_search_ablation.py results/foo.jsonl [bar.jsonl ...]

No third-party dependencies (stdlib only), matching this repo's other
scripts/ conventions.
"""
import json
import random
import statistics
import sys
from collections import defaultdict

# (arm_a, arm_b, threads_filter) -- comparison is always "b relative to a"
FIXED_DEPTH_COMPARISONS = [
    ("A", "B", {1}),
    ("B", "C", {2, 4}),
    ("C", "D", {2, 4}),
    ("B", "E", {2, 4}),
    ("D", "E", {2, 4}),
]

BOOTSTRAP_RESAMPLES = 2000
BOOTSTRAP_SEED = 20260722


def load_records(paths):
    records = []
    for path in paths:
        with open(path) as f:
            for line in f:
                line = line.strip()
                if not line:
                    continue
                records.append(json.loads(line))
    return records


def category_of(position_id):
    # "opening_001" -> "opening", "tactics_collision_031" -> "tactics_collision"
    parts = position_id.rsplit("_", 1)
    return parts[0] if len(parts) == 2 and parts[1].isdigit() else position_id


def median(values):
    return statistics.median(values) if values else 0.0


def bootstrap_ci95(deltas, seed):
    if not deltas:
        return (0.0, 0.0)
    rng = random.Random(seed)
    n = len(deltas)
    stats = []
    for _ in range(BOOTSTRAP_RESAMPLES):
        resample = [deltas[rng.randrange(n)] for _ in range(n)]
        stats.append(median(resample))
    stats.sort()
    lo_idx = int(round((n and BOOTSTRAP_RESAMPLES - 1) * 0.025))
    hi_idx = int(round((BOOTSTRAP_RESAMPLES - 1) * 0.975))
    return (stats[lo_idx], stats[hi_idx])


def paired_report(deltas, seed):
    if not deltas:
        return None
    improved = sum(1 for d in deltas if d < -1e-9)
    worsened = sum(1 for d in deltas if d > 1e-9)
    tied = len(deltas) - improved - worsened
    lo, hi = bootstrap_ci95(deltas, seed)
    return {
        "n": len(deltas),
        "median": median(deltas),
        "mean": statistics.fmean(deltas),
        "ci95": (lo, hi),
        "improved": improved,
        "worsened": worsened,
        "tied": tied,
    }


def fmt_report(label, r):
    if r is None:
        return f"{label}: no paired data"
    return (
        f"{label}: n={r['n']} median={r['median']:+.4f} mean={r['mean']:+.4f} "
        f"ci95=[{r['ci95'][0]:+.4f},{r['ci95'][1]:+.4f}] "
        f"improved={r['improved']} worsened={r['worsened']} tied={r['tied']}"
    )


def analyze_fixed_depth(records):
    recs = [r for r in records if r.get("mode") == "fixed-depth"]
    if not recs:
        print("(no fixed-depth records found)")
        return

    # key: (position_id, requested_depth, threads, profile, arm) -> list of records
    by_key = defaultdict(list)
    for r in recs:
        key = (r["position_id"], r["requested_depth"], r["threads"], r["profile"], r["arm"])
        by_key[key].append(r)

    def arm_medians(position_id, depth, threads, profile, arm):
        recs_ = by_key.get((position_id, depth, threads, profile, arm))
        if not recs_:
            return None
        return {
            "elapsed_ns": median([r["elapsed_ns"] for r in recs_]),
            "total_nodes": median([r["total_nodes"] for r in recs_]),
            "nps": median([r["nps"] for r in recs_]),
            "score": median([r["score"] for r in recs_]),
            "bestmove": statistics.mode([r["bestmove"] for r in recs_]),
        }

    positions = sorted({r["position_id"] for r in recs})
    depths = sorted({r["requested_depth"] for r in recs})
    profiles = sorted({r["profile"] for r in recs})

    for profile in profiles:
        print(f"\n=== fixed-depth paired analysis: profile={profile} ===")
        for arm_a, arm_b, thread_filter in FIXED_DEPTH_COMPARISONS:
            for threads in sorted(thread_filter):
                elapsed_deltas = []
                nodes_deltas = []
                nps_deltas = []
                score_matches = 0
                bm_matches = 0
                n_pairs = 0
                by_category = defaultdict(list)
                for pos in positions:
                    for depth in depths:
                        a = arm_medians(pos, depth, threads, profile, arm_a)
                        b = arm_medians(pos, depth, threads, profile, arm_b)
                        if a is None or b is None:
                            continue
                        n_pairs += 1
                        if a["elapsed_ns"] > 0:
                            d = (b["elapsed_ns"] - a["elapsed_ns"]) / a["elapsed_ns"]
                            elapsed_deltas.append(d)
                            by_category[category_of(pos)].append(d)
                        if a["total_nodes"] > 0:
                            nodes_deltas.append(
                                (b["total_nodes"] - a["total_nodes"]) / a["total_nodes"]
                            )
                        if a["nps"] > 0:
                            nps_deltas.append((b["nps"] - a["nps"]) / a["nps"])
                        if a["score"] == b["score"]:
                            score_matches += 1
                        if a["bestmove"] == b["bestmove"]:
                            bm_matches += 1
                if n_pairs == 0:
                    continue
                print(f"-- {arm_a} vs {arm_b}, threads={threads} --")
                print(
                    "  "
                    + fmt_report(
                        "delta_elapsed_ratio",
                        paired_report(elapsed_deltas, BOOTSTRAP_SEED ^ hash((arm_a, arm_b, threads))),
                    )
                )
                print(
                    "  "
                    + fmt_report(
                        "delta_total_nodes_ratio",
                        paired_report(
                            nodes_deltas, BOOTSTRAP_SEED ^ hash((arm_a, arm_b, threads, 1))
                        ),
                    )
                )
                print(
                    "  "
                    + fmt_report(
                        "delta_nps_ratio",
                        paired_report(nps_deltas, BOOTSTRAP_SEED ^ hash((arm_a, arm_b, threads, 2))),
                    )
                )
                print(
                    f"  score_agreement={score_matches / n_pairs:.2f} "
                    f"bestmove_agreement={bm_matches / n_pairs:.2f} (n={n_pairs} position*depth pairs)"
                )
                if by_category:
                    print("  category breakdown (median delta_elapsed_ratio):")
                    for cat in sorted(by_category):
                        print(f"    {cat}: median={median(by_category[cat]):+.4f} (n={len(by_category[cat])})")


def analyze_fixed_time(records):
    recs = [r for r in records if r.get("mode") == "fixed-time"]
    if not recs:
        print("(no fixed-time records found)")
        return

    by_key = defaultdict(list)
    for r in recs:
        key = (r["position_id"], r["threads"], r["profile"], r["arm"])
        by_key[key].append(r)

    def arm_medians(position_id, threads, profile, arm):
        recs_ = by_key.get((position_id, threads, profile, arm))
        if not recs_:
            return None
        return {
            "completed_depth": median([r["completed_depth"] for r in recs_]),
            "total_nodes": median([r["total_nodes"] for r in recs_]),
            "time_overrun_ns": median([r["time_overrun_ns"] for r in recs_ if r["time_overrun_ns"] is not None] or [0]),
            "score": median([r["score"] for r in recs_]),
            "bestmove": statistics.mode([r["bestmove"] for r in recs_]),
        }

    positions = sorted({r["position_id"] for r in recs})
    profiles = sorted({r["profile"] for r in recs})

    for profile in profiles:
        print(f"\n=== fixed-time paired analysis: profile={profile} ===")
        for arm_a, arm_b, thread_filter in FIXED_DEPTH_COMPARISONS:
            for threads in sorted(thread_filter):
                depth_deltas = []
                nodes_deltas = []
                overrun_deltas = []
                score_matches = 0
                bm_matches = 0
                n_pairs = 0
                for pos in positions:
                    a = arm_medians(pos, threads, profile, arm_a)
                    b = arm_medians(pos, threads, profile, arm_b)
                    if a is None or b is None:
                        continue
                    n_pairs += 1
                    depth_deltas.append(b["completed_depth"] - a["completed_depth"])
                    if a["total_nodes"] > 0:
                        nodes_deltas.append((b["total_nodes"] - a["total_nodes"]) / a["total_nodes"])
                    if a["time_overrun_ns"] != 0:
                        overrun_deltas.append(
                            (b["time_overrun_ns"] - a["time_overrun_ns"]) / abs(a["time_overrun_ns"])
                        )
                    if a["score"] == b["score"]:
                        score_matches += 1
                    if a["bestmove"] == b["bestmove"]:
                        bm_matches += 1
                if n_pairs == 0:
                    continue
                print(f"-- {arm_a} vs {arm_b}, threads={threads} --")
                print(
                    "  "
                    + fmt_report(
                        "delta_completed_depth (absolute)",
                        paired_report(depth_deltas, BOOTSTRAP_SEED ^ hash((arm_a, arm_b, threads, "t"))),
                    )
                )
                print(
                    "  "
                    + fmt_report(
                        "delta_total_nodes_ratio",
                        paired_report(nodes_deltas, BOOTSTRAP_SEED ^ hash((arm_a, arm_b, threads, "tn"))),
                    )
                )
                print(
                    f"  score_agreement={score_matches / n_pairs:.2f} "
                    f"bestmove_agreement={bm_matches / n_pairs:.2f} (n={n_pairs} positions)"
                )


def main():
    if len(sys.argv) < 2:
        print("usage: analyze_search_ablation.py <jsonl> [more.jsonl ...]", file=sys.stderr)
        sys.exit(1)
    records = load_records(sys.argv[1:])
    print(f"loaded {len(records)} records from {len(sys.argv) - 1} file(s)")
    analyze_fixed_depth(records)
    analyze_fixed_time(records)


if __name__ == "__main__":
    main()
