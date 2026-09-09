#!/usr/bin/env python3
"""Rank candidate NNUE audit reports without making a strength claim."""

from __future__ import annotations

import argparse
import json
from pathlib import Path


def weighted_mean_abs_delta(report: dict[str, object]) -> float:
    groups = report["groups"]
    # Each axis partitions the same rows; use the explicit side partition once.
    side = groups["side"]
    total = sum(float(item["mean_abs_delta_cp"]) * int(item["count"]) for item in side.values())
    return total / int(report["positions"])


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reports", nargs="+", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    rows = []
    for path in args.reports:
        report = json.loads(path.read_text(encoding="utf-8"))
        if not report.get("comparison_valid"):
            raise SystemExit(f"invalid comparison report: {path}")
        rows.append({
            "report": str(path),
            "candidate": str(path).rsplit("seed", 1)[-1].split("_loss_audit", 1)[0],
            "positions": report["positions"],
            "invalid_positions": len(report["invalid_positions"]),
            "candidate_score_variance_cp2": report["candidate_score_variance_cp2"],
            "weighted_mean_abs_delta_cp": round(weighted_mean_abs_delta(report), 3),
            "outliers": len(report["outliers"]),
        })
    rows.sort(key=lambda row: (row["weighted_mean_abs_delta_cp"], row["candidate"]))
    result = {
        "purpose": "compare saved NNUE candidate audit behavior",
        "strength_claim": False,
        "ranking_basis": "weighted mean absolute delta against v011 baseline audit",
        "candidates": rows,
        "recommended_for_next_review": rows[0]["candidate"],
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
