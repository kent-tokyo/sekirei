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


def score_variance(values: list[int]) -> float:
    if not values:
        return 0.0
    mean = statistics.mean(values)
    return statistics.mean((value - mean) ** 2 for value in values)


def probe(binary: Path, weights: Path, sfens: list[str], skip_invalid: bool) -> tuple[list[str], list[int], list[dict[str, str]]]:
    """Probe in batches, splitting failed batches to isolate bad SFEN rows."""
    valid_sfens: list[str] = []
    scores: list[int] = []
    invalid: list[dict[str, str]] = []

    def visit(batch: list[str]) -> None:
        if not batch:
            return
        args = [str(binary), str(weights), "--json"]
        for sfen in batch:
            args.extend(("--sfen", sfen))
        result = subprocess.run(args, capture_output=True, text=True)
        if result.returncode == 0:
            rows = json.loads(result.stdout)["probes"]
            valid_sfens.extend(sfen for sfen, _ in zip(batch, rows))
            scores.extend(int(row["score_cp"]) for row in rows)
            return
        if len(batch) > 1:
            midpoint = len(batch) // 2
            visit(batch[:midpoint])
            visit(batch[midpoint:])
            return
        error = result.stderr.strip() or "probe failed"
        invalid.append({"sfen": batch[0], "error": error})
        if not skip_invalid:
            raise subprocess.CalledProcessError(result.returncode, args, result.stdout, result.stderr)

    for start in range(0, len(sfens), 32):
        visit(sfens[start:start + 32])
    return valid_sfens, scores, invalid


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--binary", type=Path, default=Path("target/release/nnue_probe"))
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--limit", type=int, default=100)
    parser.add_argument("--outlier-percentile", type=float, default=0.95)
    parser.add_argument("--output", type=Path, help="write the JSON report to this path")
    parser.add_argument("--skip-invalid", action="store_true", help="exclude invalid SFENs and record them")
    args = parser.parse_args()
    sfens = [line.strip() for line in args.corpus.read_text().splitlines()
             if line.strip() and not line.startswith("#")][:args.limit]
    candidate_sfens, candidate_scores, candidate_invalid = probe(
        args.binary, args.candidate, sfens, args.skip_invalid
    )
    baseline_sfens, baseline_scores, baseline_invalid = probe(
        args.binary, args.baseline, candidate_sfens, args.skip_invalid
    )
    if candidate_sfens != baseline_sfens:
        raise SystemExit("candidate and baseline valid SFEN sets differ")
    sfens = baseline_sfens
    deltas = [x - y for x, y in zip(candidate_scores, baseline_scores)]
    candidate_variance = score_variance(candidate_scores)
    baseline_variance = score_variance(baseline_scores)
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
        "input_positions": len(sfens) + len(candidate_invalid),
        "invalid_positions": candidate_invalid + baseline_invalid,
        "comparison_valid": bool(rows) and candidate_variance > 0.0 and baseline_variance > 0.0,
        "comparison_invalid_reason": (
            "constant candidate or baseline output; score deltas are diagnostic only"
            if candidate_variance == 0.0 or baseline_variance == 0.0
            else None
        ),
        "candidate_score_variance_cp2": candidate_variance,
        "baseline_score_variance_cp2": baseline_variance,
        "outlier_percentile": args.outlier_percentile,
        "outlier_threshold_abs_delta_cp": threshold,
        "groups": groups,
        "outliers": sorted((row for row in rows if row["outlier"]),
                           key=lambda row: row["abs_delta_cp"], reverse=True),
    }
    rendered = json.dumps(report, ensure_ascii=False, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered, encoding="utf-8")
    print(rendered, end="")


if __name__ == "__main__":
    main()
