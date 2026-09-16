#!/usr/bin/env python3
"""Gate 2 (Official NNUE v1) — compares two analysis_record_v1 JSONL runs
of the SAME position corpus against the SAME engine binary, one with
--eval-file set (the candidate NNUE) and one without (material fallback),
per docs/experiments/official_nnue_v1_preregistration.md's Gate 2 section.

Implements exactly the metrics already defined in
docs/amateur_analysis_benchmark.md ("Metrics" section) -- no new
thresholds or formulas invented for this gate. Diagnostic only: this
script does not compute or imply any strength/Elo verdict (see that
doc's "Do not overclaim" section).

Usage:
  python3 scripts/gate2_compare_analysis_runs.py \
      --with-eval with_eval.jsonl --without-eval without_eval.jsonl \
      --output comparison.json
"""
import argparse
import json
import statistics
import sys


def load_records(path):
    """Returns {sample_id: record}. Raises if a sample_id repeats within
    one file (each position must appear exactly once per run)."""
    records = {}
    with open(path) as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            rec = json.loads(line)
            sid = rec["sample_id"]
            if sid in records:
                raise ValueError(f"{path}: duplicate sample_id {sid!r}")
            records[sid] = rec
    return records


def top1_agreement(with_eval, without_eval, shared_ids):
    """Fraction of shared, status=ok-in-both positions where the two
    runs' top-level bestmove fields match. Positions where either run's
    top-level bestmove is None (status != ok) are excluded from the
    denominator -- an agreement metric over "no move produced" is not
    meaningful, and that population is already covered by the coverage
    metric below."""
    comparable = [
        sid
        for sid in shared_ids
        if with_eval[sid]["bestmove"] is not None and without_eval[sid]["bestmove"] is not None
    ]
    if not comparable:
        return {"n": 0, "agreement": None}
    agree = sum(1 for sid in comparable if with_eval[sid]["bestmove"] == without_eval[sid]["bestmove"])
    return {"n": len(comparable), "agreement": agree / len(comparable)}


def _top3_bestmoves(rec):
    lines = sorted(rec.get("lines", []), key=lambda l: l["multipv"])[:3]
    return {l["bestmove"] for l in lines}


def top3_overlap(with_eval, without_eval, shared_ids):
    """Set overlap (Jaccard) between the two runs' first-3-by-multipv
    lines[].bestmove values, per docs/amateur_analysis_benchmark.md.
    Requires both runs to have used --multipv >= 3 (lines[] truncated to
    what was actually requested; a run with multipv=1 degenerates to a
    1-element set, which still computes a valid but less informative
    Jaccard score -- not an error, but reported denominators make this
    visible rather than silently averaging it in)."""
    comparable = [
        sid
        for sid in shared_ids
        if with_eval[sid]["status"] == "ok" and without_eval[sid]["status"] == "ok"
    ]
    if not comparable:
        return {"n": 0, "mean_jaccard": None, "exact_match_rate": None}
    jaccards = []
    exact = 0
    for sid in comparable:
        a = _top3_bestmoves(with_eval[sid])
        b = _top3_bestmoves(without_eval[sid])
        union = a | b
        jaccards.append((len(a & b) / len(union)) if union else 1.0)
        if a == b:
            exact += 1
    return {
        "n": len(comparable),
        "mean_jaccard": statistics.mean(jaccards),
        "exact_match_rate": exact / len(comparable),
    }


def score_cp_shift(with_eval, without_eval, shared_ids):
    """Within-engine score_cp delta only (with_eval - without_eval, same
    binary, same position, only EvalFile differs) -- per
    docs/amateur_analysis_benchmark.md's "Raw CP is not comparable across
    engines" rule, this is the one comparison that rule explicitly
    permits, since both sides are the same engine. Only over the
    top-level (multipv=1) line's score_cp; positions where either side's
    top line reports score_mate instead are excluded here and counted
    separately by mate_agreement below -- a cp delta against a mate score
    is not a number."""
    deltas = []
    excluded_mate = 0
    for sid in shared_ids:
        we, woe = with_eval[sid], without_eval[sid]
        if we["status"] != "ok" or woe["status"] != "ok":
            continue
        we_top = next((l for l in we["lines"] if l["multipv"] == 1), None)
        woe_top = next((l for l in woe["lines"] if l["multipv"] == 1), None)
        if we_top is None or woe_top is None:
            continue
        if "score_cp" not in we_top or "score_cp" not in woe_top:
            excluded_mate += 1
            continue
        deltas.append(we_top["score_cp"] - woe_top["score_cp"])
    if not deltas:
        return {"n": 0, "excluded_mate": excluded_mate, "mean": None, "median": None, "stdev": None}
    return {
        "n": len(deltas),
        "excluded_mate": excluded_mate,
        "mean": statistics.mean(deltas),
        "median": statistics.median(deltas),
        "stdev": statistics.stdev(deltas) if len(deltas) >= 2 else 0.0,
    }


def mate_agreement(with_eval, without_eval, shared_ids):
    """Compares score_mate fields directly, only where BOTH sides'
    top-line report a mate score (never compared against a cp record,
    per the doc's explicit warning)."""
    comparable = []
    for sid in shared_ids:
        we, woe = with_eval[sid], without_eval[sid]
        if we["status"] != "ok" or woe["status"] != "ok":
            continue
        we_top = next((l for l in we["lines"] if l["multipv"] == 1), None)
        woe_top = next((l for l in woe["lines"] if l["multipv"] == 1), None)
        if we_top is None or woe_top is None:
            continue
        if "score_mate" in we_top and "score_mate" in woe_top:
            comparable.append(we_top["score_mate"] == woe_top["score_mate"])
    if not comparable:
        return {"n": 0, "agreement": None}
    return {"n": len(comparable), "agreement": sum(comparable) / len(comparable)}


def coverage(records, label):
    """count(status != ok) / total, broken down by the 4-way status enum."""
    total = len(records)
    counts = {"ok": 0, "timeout": 0, "incomplete": 0, "engine_error": 0}
    for rec in records.values():
        counts[rec["status"]] = counts.get(rec["status"], 0) + 1
    return {
        "label": label,
        "total": total,
        "counts": counts,
        "non_ok_rate": (total - counts["ok"]) / total if total else None,
    }


def compare(with_eval_path, without_eval_path):
    with_eval = load_records(with_eval_path)
    without_eval = load_records(without_eval_path)
    with_ids, without_ids = set(with_eval), set(without_eval)
    shared_ids = with_ids & without_ids
    only_with = with_ids - without_ids
    only_without = without_ids - with_ids

    return {
        "corpus_size": {
            "with_eval": len(with_eval),
            "without_eval": len(without_eval),
            "shared": len(shared_ids),
            "only_in_with_eval": sorted(only_with),
            "only_in_without_eval": sorted(only_without),
        },
        "coverage": [coverage(with_eval, "with_eval"), coverage(without_eval, "without_eval")],
        "top1_agreement": top1_agreement(with_eval, without_eval, shared_ids),
        "top3_overlap": top3_overlap(with_eval, without_eval, shared_ids),
        "score_cp_shift": score_cp_shift(with_eval, without_eval, shared_ids),
        "mate_agreement": mate_agreement(with_eval, without_eval, shared_ids),
    }


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--with-eval", required=True, help="analysis_record_v1 JSONL, --eval-file run")
    ap.add_argument("--without-eval", required=True, help="analysis_record_v1 JSONL, material-fallback run")
    ap.add_argument("--output", required=True, help="comparison JSON output path")
    args = ap.parse_args()

    result = compare(args.with_eval, args.without_eval)
    with open(args.output, "w") as f:  # lgtm[py/path-injection] -- CLI-provided output path, not derived from untrusted input
        json.dump(result, f, indent=2)
        f.write("\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
