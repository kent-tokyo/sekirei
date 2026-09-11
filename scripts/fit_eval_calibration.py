#!/usr/bin/env python3
"""Fit a diagnostic affine calibration from NNUE scores to material scores."""

from __future__ import annotations

import argparse
import json
import math
import re
import statistics
from pathlib import Path


ROW = re.compile(r"material_cp=(-?\d+) nnue_cp=(-?\d+) delta_cp=(-?\d+)")


def load_rows(path: Path) -> list[tuple[float, float]]:
    rows = []
    for line in path.read_text(encoding="utf-8").splitlines():
        match = ROW.search(line)
        if match:
            material, nnue, _ = map(float, match.groups())
            rows.append((nnue, material))
    if not rows:
        raise ValueError(f"no compare_eval rows found in {path}")
    return rows


def mae(rows: list[tuple[float, float]], slope: float, intercept: float) -> float:
    return statistics.fmean(abs(slope * nnue + intercept - material) for nnue, material in rows)


def fit(rows: list[tuple[float, float]]) -> dict[str, object]:
    x_mean = statistics.fmean(nnue for nnue, _ in rows)
    y_mean = statistics.fmean(material for _, material in rows)
    denominator = sum((nnue - x_mean) ** 2 for nnue, _ in rows)
    slope = sum((nnue - x_mean) * (material - y_mean) for nnue, material in rows) / denominator if denominator else 0.0
    intercept = y_mean - slope * x_mean
    before = mae(rows, 0.0, 0.0)
    after = mae(rows, slope, intercept)
    return {
        "schema": "sekirei.eval-calibration.v1",
        "positions": len(rows),
        "slope": round(slope, 8),
        "intercept_cp": round(intercept, 3),
        "mae_before_cp": round(before, 3),
        "mae_after_cp": round(after, 3),
        "mae_improvement_ratio": round((before - after) / before, 6) if before else 0.0,
        "usable_for_engine": bool(
            math.isfinite(slope)
            and math.isfinite(intercept)
            and abs(slope) > 1e-9
            and after < before
        ),
        "strength_claim": False,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    report = fit(load_rows(args.input))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
