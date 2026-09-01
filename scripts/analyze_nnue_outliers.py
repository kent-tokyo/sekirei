#!/usr/bin/env python3
"""Classify large NNUE deltas on a fixed SFEN corpus.

This is a structural diagnostic: the groups describe board features, not
playing strength or a claim that a group is intrinsically wrong.  The score
delta is candidate minus baseline, and the outlier threshold is the requested
absolute percentile of that delta.
"""

import argparse
import json
import statistics
import subprocess
from collections import defaultdict
from pathlib import Path


def features(sfen: str) -> dict[str, str | int]:
    board, side, hand, ply = sfen.split()[:4]
    ranks = board.split("/")
    pieces = [c for c in board if c.isalpha()]
    promoted = board.count("+")
    hand_nonempty = hand != "-"
    material = sum(1 for c in pieces if c.isupper()) - sum(1 for c in pieces if c.islower())
    return {
        "side": side,
        "hand": "with_hand" if hand_nonempty else "no_hand",
        "promotion": "with_promotion" if promoted else "no_promotion",
        "phase": (
            "early" if int(ply) < 20 else
            "middle" if int(ply) < 40 else
            "late"
        ),
        "material": "white_ahead" if material < 0 else "black_ahead" if material > 0 else "even",
        "piece_count": len(pieces),
        "ply": int(ply),
        "promoted_count": promoted,
        "sfen": sfen,
    }


def percentile(values: list[int], fraction: float) -> int:
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, int(fraction * (len(ordered) - 1)))]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--binary", type=Path, default=Path("target/release/nnue_probe"))
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--limit", type=int, default=100)
    parser.add_argument("--outlier-percentile", type=float, default=0.95)
    args = parser.parse_args()
    sfens = [line.strip() for line in args.corpus.read_text().splitlines()
             if line.strip() and not line.startswith("#")][:args.limit]
    probe_args = [str(args.binary), "--json"]
    for sfen in sfens:
        probe_args.extend(("--sfen", sfen))
    candidate = subprocess.run([*probe_args[:1], str(args.candidate), *probe_args[1:]],
                               check=True, capture_output=True, text=True)
    baseline = subprocess.run([*probe_args[:1], str(args.baseline), *probe_args[1:]],
                              check=True, capture_output=True, text=True)
    candidate_scores = [row["score_cp"] for row in json.loads(candidate.stdout)["probes"]]
    baseline_scores = [row["score_cp"] for row in json.loads(baseline.stdout)["probes"]]
    deltas = [x - y for x, y in zip(candidate_scores, baseline_scores)]
    threshold = percentile([abs(int(x)) for x in deltas], args.outlier_percentile)
    rows = []
    for sfen, delta in zip(sfens, deltas):
        row = features(sfen)
        row["delta_cp"] = int(delta)
        row["abs_delta_cp"] = abs(int(delta))
        row["outlier"] = abs(int(delta)) >= threshold
        rows.append(row)

    groups = {}
    for key in ("side", "hand", "promotion", "phase", "material"):
        grouped = defaultdict(list)
        for row in rows:
            grouped[row[key]].append(row["abs_delta_cp"])
        groups[key] = {
            value: {
                "count": len(values),
                "mean_abs_delta_cp": round(statistics.mean(values), 3),
                "max_abs_delta_cp": max(values),
                "outliers": sum(1 for row in rows if row[key] == value and row["outlier"]),
            }
            for value, values in sorted(grouped.items())
        }
    report = {
        "positions": len(rows),
        "outlier_percentile": args.outlier_percentile,
        "outlier_threshold_abs_delta_cp": threshold,
        "groups": groups,
        "outliers": sorted((row for row in rows if row["outlier"]),
                           key=lambda row: row["abs_delta_cp"], reverse=True),
    }
    print(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
