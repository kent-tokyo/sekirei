#!/usr/bin/env python3
"""Evaluate a material-calibration pilot on frozen C5c-disjoint positions.

This is a score-calibration diagnostic.  It deliberately makes no strength or
candidate-adoption claim, even when the pre-registered compression metric
improves.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import math
import statistics
from pathlib import Path


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("fixed", ROOT / "run_fixed_selfplay_diagnostic.py")
assert SPEC and SPEC.loader
FIXED = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(FIXED)
MATE_ABS_MIN = 899_000


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_rows(path: Path, limit: int, offset: int) -> list[dict]:
    rows = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
    if not rows:
        raise ValueError("hold-out is empty")
    rows.sort(key=lambda row: hashlib.sha256((row["source"]["path"] + "\0" + row["sfen"]).encode()).hexdigest())
    if offset < 0:
        raise ValueError("offset must not be negative")
    selected = rows[offset : offset + limit]
    if len(selected) != limit:
        raise ValueError("hold-out does not contain enough positions for offset + limit")
    return selected


def complete(result: dict) -> bool:
    return result.get("completed_iteration_valid") is True and result.get("completed_bound") == "exact" and result.get("pv_legal") is True


def cp_comparable(row: dict) -> bool:
    return all(complete(row[name]) and abs(row[name]["score_cp"]) < MATE_ABS_MIN for name in ("material", "baseline", "candidate"))


def pearson(left: list[float], right: list[float]) -> float | None:
    if len(left) < 2:
        return None
    left_mean, right_mean = statistics.fmean(left), statistics.fmean(right)
    left_var = sum((value - left_mean) ** 2 for value in left)
    right_var = sum((value - right_mean) ** 2 for value in right)
    if left_var == 0 or right_var == 0:
        return None
    return sum((a - left_mean) * (b - right_mean) for a, b in zip(left, right, strict=True)) / math.sqrt(left_var * right_var)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--holdout", type=Path, required=True)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--nodes", type=int, default=100_000)
    parser.add_argument("--limit", type=int, default=128)
    parser.add_argument("--offset", type=int, default=0, help="skip this many deterministically ordered hold-out positions")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.nodes <= 0 or args.limit <= 0 or args.offset < 0:
        parser.error("nodes and limit must be positive and offset must not be negative")
    rows = load_rows(args.holdout, args.limit, args.offset)
    results = []
    for index, row in enumerate(rows, 1):
        diagnostic_row = {"pre_move_sfen": row["sfen"]}
        material = FIXED.run_one(args.engine, diagnostic_row, nodes=args.nodes, weights=None, root_move=None)
        baseline = FIXED.run_one(args.engine, diagnostic_row, nodes=args.nodes, weights=args.baseline, root_move=None)
        candidate = FIXED.run_one(args.engine, diagnostic_row, nodes=args.nodes, weights=args.candidate, root_move=None)
        results.append({"id": f"holdout-{index:03d}", "source": row["source"], "tags": row.get("tags", {}), "material": material, "baseline": baseline, "candidate": candidate})
    completed = [row for row in results if all(complete(row[name]) for name in ("material", "baseline", "candidate"))]
    valid = [row for row in completed if cp_comparable(row)]
    mate_like = [row for row in completed if row not in valid]
    anchors = [row for row in valid if abs(row["material"]["score_cp"]) >= 1_000]
    base_compressed = [row for row in anchors if abs(row["baseline"]["score_cp"]) <= 100]
    candidate_compressed = [row for row in anchors if abs(row["candidate"]["score_cp"]) <= 100]
    candidate_closer = [row for row in valid if abs(row["candidate"]["score_cp"] - row["material"]["score_cp"]) < abs(row["baseline"]["score_cp"] - row["material"]["score_cp"])]
    material_scores = [float(row["material"]["score_cp"]) for row in valid]
    baseline_scores = [float(row["baseline"]["score_cp"]) for row in valid]
    candidate_scores = [float(row["candidate"]["score_cp"]) for row in valid]
    document = {
        "schema": "sekirei.selfplay-calibration-holdout.v1", "diagnostic_only": True, "strength_claim": False,
        "contract": {"nodes": args.nodes, "threads": 1, "spec_top_n": 0, "tt": "cold_process_per_search", "nnue_output": "absolute", "selection": "SHA-256(source.path + NUL + SFEN), ordered offset then limit", "offset": args.offset, "limit": args.limit},
        "inputs": {"holdout": str(args.holdout), "holdout_sha256": sha256(args.holdout), "engine": str(args.engine), "engine_sha256": sha256(args.engine), "baseline": str(args.baseline), "baseline_sha256": sha256(args.baseline), "candidate": str(args.candidate), "candidate_sha256": sha256(args.candidate)},
        "results": results,
        "summary": {
            "selected": len(results), "complete": len(completed), "incomplete": len(results) - len(completed), "cp_comparable": len(valid), "mate_like_excluded": len(mate_like), "mate_abs_min": MATE_ABS_MIN, "material_anchors_abs_ge_1000": len(anchors),
            "baseline_compressed_abs_le_100": len(base_compressed), "candidate_compressed_abs_le_100": len(candidate_compressed),
            "candidate_closer_to_material": len(candidate_closer), "baseline_material_pearson": pearson(baseline_scores, material_scores), "candidate_material_pearson": pearson(candidate_scores, material_scores),
            "pre_registered_calibration_signal": "candidate compressed count must be strictly lower than baseline on completed material-anchor rows",
            "signal_pass": len(candidate_compressed) < len(base_compressed),
            "interpretation": "A pass only supports this material-anchor calibration diagnostic. It does not establish material correctness, playing strength, or candidate adoption.",
        },
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(document["summary"], sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
