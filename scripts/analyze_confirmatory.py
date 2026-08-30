#!/usr/bin/env python3
"""Confirmatory-ablation analysis: position-level paired comparisons with
pre-registered decision rules, over search_ablation JSONL output.

Aggregation discipline (per the confirmatory-ablation spec): repetitions
are collapsed to ONE value per (position, depth-or-time-mode, threads,
profile, arm) group FIRST (median across reps), and per-position
coefficient of variation is reported alongside as a repeatability check.
Cross-arm comparison bootstrapping resamples POSITIONS, never individual
repetitions.

Metric direction matters and is NOT the same for every field:
  - elapsed, total_nodes, time_overrun: LOWER is better
  - completed_depth: HIGHER is better
This script tracks direction explicitly per metric instead of a single
"negative = improved" convention, which a previous version of this
analysis got backwards for completed_depth.

"On-vs-on baseline" for arm D comparisons: rather than a separate run,
this is estimated from the SAME dataset's own per-group repetition scatter
-- the fraction of repetitions agreeing with that group's own modal
bestmove is exactly the self-agreement rate a dedicated on-vs-on rerun
would also measure, since both isolate "how often does this exact
(position, depth, threads, arm) config land on the same answer across
independent fresh-state runs" from any cross-arm algorithmic difference.

Usage:
    python3 scripts/analyze_confirmatory.py <jsonl> [more.jsonl ...]

stdlib only.
"""
import json
import random
import statistics
import sys
from collections import defaultdict

BOOTSTRAP_RESAMPLES = 2000
BOOTSTRAP_SEED = 20260722

# (arm_a, arm_b, threads, requires_note_on_nodes)
FIXED_DEPTH_COMPARISONS = [
    ("A", "B", {1}),
    ("B", "C", {2, 4}),
    ("C", "D", {2, 4}),
    ("B", "E", {2, 4}),
    ("D", "E", {2, 4}),
]


def load_records(paths):
    records = []
    for path in paths:
        with open(path) as f:
            for line in f:
                line = line.strip()
                if line:
                    records.append(json.loads(line))
    return records


def category_of(position_id):
    parts = position_id.rsplit("_", 1)
    return parts[0] if len(parts) == 2 and parts[1].isdigit() else position_id


def median(values):
    return statistics.median(values) if values else 0.0


def mean(values):
    return statistics.fmean(values) if values else 0.0


def cv(values):
    if not values or mean(values) == 0:
        return 0.0
    m = mean(values)
    if len(values) < 2:
        return 0.0
    return statistics.stdev(values) / abs(m)


def mode(values):
    return statistics.mode(values)


def self_agreement(values):
    """Fraction of `values` equal to the group's own mode -- the group's
    internal repeatability rate, used as the on-vs-on baseline proxy."""
    if not values:
        return 1.0
    m = mode(values)
    return sum(1 for v in values if v == m) / len(values)


def bootstrap_ci95(values, statistic, seed):
    if not values:
        return (0.0, 0.0)
    rng = random.Random(seed)
    n = len(values)
    stats = []
    for _ in range(BOOTSTRAP_RESAMPLES):
        resample = [values[rng.randrange(n)] for _ in range(n)]
        stats.append(statistic(resample))
    stats.sort()
    lo_idx = int(round((BOOTSTRAP_RESAMPLES - 1) * 0.025))
    hi_idx = int(round((BOOTSTRAP_RESAMPLES - 1) * 0.975))
    return (stats[lo_idx], stats[hi_idx])


class GroupSummary:
    """One (position, depth-or-mode, threads, profile, arm) group, reps collapsed."""

    def __init__(self, recs):
        self.n_reps = len(recs)
        self.elapsed_ns = median([r["elapsed_ns"] for r in recs])
        self.elapsed_cv = cv([r["elapsed_ns"] for r in recs])
        self.total_nodes = median([r["total_nodes"] for r in recs])
        self.completed_depth = median([r["completed_depth"] for r in recs])
        self.score = mode([r["score"] for r in recs])
        self.bestmove = mode([r["bestmove"] for r in recs])
        self.bestmove_self_agreement = self_agreement([r["bestmove"] for r in recs])
        self.score_self_agreement = self_agreement([r["score"] for r in recs])
        overruns = [r["time_overrun_ns"] for r in recs if r.get("time_overrun_ns") is not None]
        self.time_overrun_ns = median(overruns) if overruns else None
        self.pv_legal_all = all(r["pv_legal"] for r in recs)
        self.state_unchanged_all = all(r["board_unchanged"] for r in recs)


def build_groups(records, mode_filter, key_fields):
    groups = defaultdict(list)
    for r in records:
        if r.get("mode") != mode_filter:
            continue
        key = tuple(r[f] for f in key_fields)
        groups[key].append(r)
    return {k: GroupSummary(v) for k, v in groups.items()}


class PairedComparison:
    def __init__(self):
        self.elapsed_deltas = []  # (b - a) / a, lower is better
        self.nodes_deltas = []
        self.depth_deltas = []  # b - a, higher is better
        self.overrun_deltas = []
        self.score_matches = 0
        self.bestmove_matches = 0
        self.n = 0
        self.by_category = defaultdict(list)  # category -> list of elapsed_deltas (or depth_deltas)
        self.a_cv = []
        self.b_cv = []
        self.a_self_agree = []
        self.b_self_agree = []
        self.pv_ok = True
        self.state_ok = True
        self.uses_speculation = False

    def add(self, a: GroupSummary, b: GroupSummary, category, primary_is_depth):
        self.n += 1
        if a.elapsed_ns > 0:
            self.elapsed_deltas.append((b.elapsed_ns - a.elapsed_ns) / a.elapsed_ns)
        if a.total_nodes > 0:
            self.nodes_deltas.append((b.total_nodes - a.total_nodes) / a.total_nodes)
        self.depth_deltas.append(b.completed_depth - a.completed_depth)
        if a.time_overrun_ns not in (None, 0) and b.time_overrun_ns is not None:
            self.overrun_deltas.append((b.time_overrun_ns - a.time_overrun_ns) / abs(a.time_overrun_ns))
        if a.score == b.score:
            self.score_matches += 1
        if a.bestmove == b.bestmove:
            self.bestmove_matches += 1
        primary_delta = self.depth_deltas[-1] if primary_is_depth else self.elapsed_deltas[-1]
        self.by_category[category].append(primary_delta)
        self.a_cv.append(a.elapsed_cv)
        self.b_cv.append(b.elapsed_cv)
        self.a_self_agree.append(a.bestmove_self_agreement)
        self.b_self_agree.append(b.bestmove_self_agreement)
        self.pv_ok &= a.pv_legal_all and b.pv_legal_all
        self.state_ok &= a.state_unchanged_all and b.state_unchanged_all


def report_metric(name, values, seed):
    if not values:
        print(f"    {name}: no data")
        return None
    lo, hi = bootstrap_ci95(values, median, seed)
    improved = sum(1 for v in values if v < -1e-9)
    worsened = sum(1 for v in values if v > 1e-9)
    tied = len(values) - improved - worsened
    print(
        f"    {name}: n={len(values)} median={median(values):+.4f} mean={mean(values):+.4f} "
        f"ci95=[{lo:+.4f},{hi:+.4f}] neg={improved} pos={worsened} tied={tied}"
    )
    return {"n": len(values), "median": median(values), "ci95": (lo, hi)}


def analyze_fixed_depth(records):
    groups = build_groups(
        records,
        "fixed-depth",
        ["position_id", "requested_depth", "threads", "profile", "arm"],
    )
    if not groups:
        print("(no fixed-depth records)")
        return
    positions_by_cat = defaultdict(set)
    for (pos, depth, threads, profile, arm) in groups:
        positions_by_cat[category_of(pos)].add(pos)

    all_keys = set(groups.keys())
    positions = sorted({k[0] for k in all_keys})
    depths = sorted({k[1] for k in all_keys})
    profiles = sorted({k[3] for k in all_keys})

    for profile in profiles:
        print(f"\n{'=' * 70}\nfixed-depth confirmatory analysis: profile={profile}\n{'=' * 70}")
        for arm_a, arm_b, thread_filter in FIXED_DEPTH_COMPARISONS:
            for threads in sorted(thread_filter):
                pc = PairedComparison()
                pc.uses_speculation = arm_b in ("D", "E") or arm_a in ("D", "E")
                for pos in positions:
                    for depth in depths:
                        ga = groups.get((pos, depth, threads, profile, arm_a))
                        gb = groups.get((pos, depth, threads, profile, arm_b))
                        if ga is None or gb is None:
                            continue
                        pc.add(ga, gb, category_of(pos), primary_is_depth=False)
                if pc.n == 0:
                    continue
                print(f"\n-- {arm_a} vs {arm_b}, threads={threads} (n={pc.n} position*depth units) --")
                seed_base = BOOTSTRAP_SEED ^ hash((arm_a, arm_b, threads, profile))
                report_metric("delta_elapsed_ratio (lower better)", pc.elapsed_deltas, seed_base)
                if pc.uses_speculation:
                    print(
                        "    delta_total_nodes_ratio: SKIPPED as primary metric -- "
                        "one arm uses speculation, whose total_nodes includes concurrent "
                        "background work, not comparable to a non-speculation arm's nodes"
                    )
                else:
                    report_metric("delta_total_nodes_ratio", pc.nodes_deltas, seed_base ^ 1)
                print(
                    f"    score_agreement={pc.score_matches / pc.n:.2f} "
                    f"bestmove_agreement={pc.bestmove_matches / pc.n:.2f}"
                )
                print(
                    f"    per-position elapsed CV: {arm_a}_mean={mean(pc.a_cv):.4f} "
                    f"{arm_b}_mean={mean(pc.b_cv):.4f}"
                )
                if pc.uses_speculation:
                    print(
                        f"    on-vs-on baseline bestmove self-agreement (repeatability, not "
                        f"cross-arm): {arm_a}={mean(pc.a_self_agree):.2f} {arm_b}={mean(pc.b_self_agree):.2f} "
                        f"-- compare against the cross-arm bestmove_agreement above to judge "
                        f"whether disagreement exceeds each arm's own known noise floor"
                    )
                print(f"    pv_legal_all={pc.pv_ok} state_unchanged_all={pc.state_ok}")
                print("    category breakdown (median elapsed delta):")
                for cat in sorted(pc.by_category):
                    vals = pc.by_category[cat]
                    print(f"      {cat}: median={median(vals):+.4f} (n={len(vals)})")


def analyze_fixed_time(records):
    groups = build_groups(records, "fixed-time", ["position_id", "threads", "profile", "arm"])
    if not groups:
        print("(no fixed-time records)")
        return
    positions = sorted({k[0] for k in groups})
    profiles = sorted({k[2] for k in groups})

    for profile in profiles:
        print(f"\n{'=' * 70}\nfixed-time confirmatory analysis: profile={profile}\n{'=' * 70}")
        for arm_a, arm_b, thread_filter in FIXED_DEPTH_COMPARISONS:
            for threads in sorted(thread_filter):
                pc = PairedComparison()
                pc.uses_speculation = arm_b in ("D", "E") or arm_a in ("D", "E")
                for pos in positions:
                    ga = groups.get((pos, threads, profile, arm_a))
                    gb = groups.get((pos, threads, profile, arm_b))
                    if ga is None or gb is None:
                        continue
                    pc.add(ga, gb, category_of(pos), primary_is_depth=True)
                if pc.n == 0:
                    continue
                print(f"\n-- {arm_a} vs {arm_b}, threads={threads} (n={pc.n} positions) --")
                seed_base = BOOTSTRAP_SEED ^ hash((arm_a, arm_b, threads, profile, "t"))
                report_metric("delta_completed_depth (higher better)", pc.depth_deltas, seed_base)
                wins = sum(1 for d in pc.depth_deltas if d > 1e-9)
                losses = sum(1 for d in pc.depth_deltas if d < -1e-9)
                ties = pc.n - wins - losses
                print(f"    completed_depth win/tie/loss ({arm_b} vs {arm_a}): {wins}/{ties}/{losses}")
                report_metric("delta_time_overrun_ratio (lower better)", pc.overrun_deltas, seed_base ^ 1)
                print(
                    f"    score_agreement={pc.score_matches / pc.n:.2f} "
                    f"bestmove_agreement={pc.bestmove_matches / pc.n:.2f}"
                )
                if not pc.uses_speculation:
                    report_metric("delta_total_nodes_ratio (auxiliary)", pc.nodes_deltas, seed_base ^ 2)
                else:
                    print("    delta_total_nodes_ratio: auxiliary only, includes speculative background work")
                if pc.uses_speculation:
                    print(
                        f"    on-vs-on baseline bestmove self-agreement: {arm_a}={mean(pc.a_self_agree):.2f} "
                        f"{arm_b}={mean(pc.b_self_agree):.2f}"
                    )
                print(f"    pv_legal_all={pc.pv_ok} state_unchanged_all={pc.state_ok}")


def main():
    if len(sys.argv) < 2:
        print("usage: analyze_confirmatory.py <jsonl> [more.jsonl ...]", file=sys.stderr)
        sys.exit(1)
    records = load_records(sys.argv[1:])
    print(f"loaded {len(records)} records from {len(sys.argv) - 1} file(s)")
    analyze_fixed_depth(records)
    analyze_fixed_time(records)


if __name__ == "__main__":
    main()
