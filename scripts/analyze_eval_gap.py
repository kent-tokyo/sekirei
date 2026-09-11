#!/usr/bin/env python3
"""Summarize static NNUE/material gaps emitted by compare_eval."""

from __future__ import annotations

import argparse
import json
import re
import statistics
from pathlib import Path


ROW = re.compile(r"material_cp=(-?\d+) nnue_cp=(-?\d+) delta_cp=(-?\d+)")


def load_rows(path: Path) -> list[tuple[int, int, int]]:
    rows = []
    for line in path.read_text(encoding="utf-8").splitlines():
        match = ROW.search(line)
        if match:
            rows.append(tuple(map(int, match.groups())))
    if not rows:
        raise ValueError(f"no compare_eval rows found in {path}")
    return rows


def summarize(rows: list[tuple[int, int, int]]) -> dict[str, object]:
    materials = [row[0] for row in rows]
    nnues = [row[1] for row in rows]
    deltas = [row[2] for row in rows]
    material_mean = statistics.fmean(materials)
    nnue_mean = statistics.fmean(nnues)
    material_std = statistics.pstdev(materials)
    nnue_std = statistics.pstdev(nnues)
    covariance = statistics.fmean(
        (material - material_mean) * (nnue - nnue_mean)
        for material, nnue in zip(materials, nnues)
    )
    correlation = covariance / (material_std * nnue_std) if material_std and nnue_std else 0.0
    return {
        "schema": "sekirei.eval-gap.v1",
        "positions": len(rows),
        "material_mean_cp": round(material_mean, 3),
        "nnue_mean_cp": round(nnue_mean, 3),
        "delta_mean_cp": round(statistics.fmean(deltas), 3),
        "delta_std_cp": round(statistics.pstdev(deltas), 3),
        "delta_max_abs_cp": max(abs(delta) for delta in deltas),
        "material_nnue_correlation": round(correlation, 6),
        "material_extreme_positions": sum(abs(material) >= 3000 for material in materials),
        "extreme_untracked_positions": sum(
            abs(material) >= 3000 and abs(nnue) < 1000
            for material, nnue in zip(materials, nnues)
        ),
        "strength_claim": False,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    report = summarize(load_rows(args.input))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
