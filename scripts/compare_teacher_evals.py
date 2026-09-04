#!/usr/bin/env python3
"""Compare material and fixed-NNUE teacher searches on the same positions.

The inputs are ``analysis_record_v1`` JSONL files produced by
``usi_analysis_export.py``.  Score statistics use only shared, successful
records whose first MultiPV line has a centipawn score.  Search-cost ratios use
all shared successful records and the first line's aggregate search counters;
the counters are repeated on every MultiPV line and must not be summed.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import statistics
from pathlib import Path
from typing import Any


SCHEMA_VERSION = 1


def read_records(path: Path) -> dict[str, dict[str, Any]]:
    records: dict[str, dict[str, Any]] = {}
    for line_number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not raw.strip():
            continue
        try:
            record = json.loads(raw)
        except json.JSONDecodeError as error:
            raise ValueError(f"{path}:{line_number}: invalid JSON: {error}") from error
        sample_id = record.get("sample_id")
        if not isinstance(sample_id, str) or not sample_id:
            raise ValueError(f"{path}:{line_number}: sample_id must be a non-empty string")
        if sample_id in records:
            raise ValueError(f"{path}:{line_number}: duplicate sample_id {sample_id!r}")
        records[sample_id] = record
    return records


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def first_line(record: dict[str, Any]) -> dict[str, Any] | None:
    lines = record.get("lines")
    if not isinstance(lines, list) or not lines or not isinstance(lines[0], dict):
        return None
    return lines[0]


def cp_score(record: dict[str, Any]) -> float | None:
    line = first_line(record)
    if line is None:
        return None
    value = line.get("score_cp")
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    value = float(value)
    return value if math.isfinite(value) else None


def sign(value: float) -> int:
    return (value > 0.0) - (value < 0.0)


def distribution(values: list[float]) -> dict[str, float]:
    if not values:
        raise ValueError("cannot summarize an empty score set")
    mean = statistics.fmean(values)
    variance = statistics.fmean((value - mean) ** 2 for value in values)
    return {
        "mean_cp": mean,
        "variance_cp2": variance,
        "stdev_cp": math.sqrt(variance),
        "min_cp": min(values),
        "max_cp": max(values),
    }


def pearson(xs: list[float], ys: list[float]) -> float | None:
    if len(xs) != len(ys) or len(xs) < 2:
        return None
    x_mean = statistics.fmean(xs)
    y_mean = statistics.fmean(ys)
    x_var_sum = sum((value - x_mean) ** 2 for value in xs)
    y_var_sum = sum((value - y_mean) ** 2 for value in ys)
    if x_var_sum == 0.0 or y_var_sum == 0.0:
        return None
    covariance_sum = sum(
        (x - x_mean) * (y - y_mean) for x, y in zip(xs, ys, strict=True)
    )
    return covariance_sum / math.sqrt(x_var_sum * y_var_sum)


def safe_ratio(numerator: float, denominator: float) -> float | None:
    return numerator / denominator if denominator > 0.0 else None


def cost_summary(records: list[dict[str, Any]]) -> dict[str, float | int]:
    nodes: list[float] = []
    time_ms: list[float] = []
    wall_ms: list[float] = []
    for record in records:
        line = first_line(record)
        if line is None:
            raise ValueError("successful record is missing its first analysis line")
        for source, key, target in (
            (line, "nodes", nodes),
            (line, "time_ms", time_ms),
            (record, "wall_time_ms", wall_ms),
        ):
            value = source.get(key)
            if isinstance(value, bool) or not isinstance(value, (int, float)):
                raise ValueError(f"successful record has invalid {key}")
            target.append(float(value))
    count = len(records)
    return {
        "positions": count,
        "nodes_total": int(sum(nodes)),
        "nodes_mean": statistics.fmean(nodes),
        "search_time_ms_total": sum(time_ms),
        "search_time_ms_mean": statistics.fmean(time_ms),
        "wall_time_ms_total": sum(wall_ms),
        "wall_time_ms_mean": statistics.fmean(wall_ms),
    }


def compare(material_path: Path, nnue_path: Path) -> dict[str, Any]:
    material = read_records(material_path)
    nnue = read_records(nnue_path)
    shared_ids = sorted(material.keys() & nnue.keys())
    shared_ok_ids = [
        sample_id
        for sample_id in shared_ids
        if material[sample_id].get("status") == "ok"
        and nnue[sample_id].get("status") == "ok"
    ]
    score_ids = [
        sample_id
        for sample_id in shared_ok_ids
        if cp_score(material[sample_id]) is not None
        and cp_score(nnue[sample_id]) is not None
    ]
    material_scores = [cp_score(material[sample_id]) for sample_id in score_ids]
    nnue_scores = [cp_score(nnue[sample_id]) for sample_id in score_ids]
    # The filtering above proves these are floats; keep the assertion local so
    # malformed records fail before any statistic is emitted.
    assert all(value is not None for value in material_scores)
    assert all(value is not None for value in nnue_scores)
    material_values = [float(value) for value in material_scores]
    nnue_values = [float(value) for value in nnue_scores]
    if not material_values:
        raise ValueError("no shared successful centipawn records")

    sign_mismatches = sum(
        sign(left) != sign(right)
        for left, right in zip(material_values, nnue_values, strict=True)
    )
    opposite_nonzero = sum(
        sign(left) * sign(right) == -1
        for left, right in zip(material_values, nnue_values, strict=True)
    )
    absolute_differences = [
        abs(left - right)
        for left, right in zip(material_values, nnue_values, strict=True)
    ]

    material_cost = cost_summary([material[sample_id] for sample_id in shared_ok_ids])
    nnue_cost = cost_summary([nnue[sample_id] for sample_id in shared_ok_ids])
    return {
        "schema_version": SCHEMA_VERSION,
        "inputs": {
            "material": {
                "path": str(material_path),
                "sha256": sha256(material_path),
                "records": len(material),
            },
            "fixed_nnue": {
                "path": str(nnue_path),
                "sha256": sha256(nnue_path),
                "records": len(nnue),
            },
        },
        "coverage": {
            "shared_records": len(shared_ids),
            "shared_ok_records": len(shared_ok_ids),
            "shared_cp_records": len(score_ids),
            "material_only": len(material.keys() - nnue.keys()),
            "fixed_nnue_only": len(nnue.keys() - material.keys()),
        },
        "scores": {
            "positions": len(score_ids),
            "pearson_correlation": pearson(material_values, nnue_values),
            "sign_mismatch_count": sign_mismatches,
            "sign_mismatch_rate": sign_mismatches / len(score_ids),
            "opposite_nonzero_sign_count": opposite_nonzero,
            "opposite_nonzero_sign_rate": opposite_nonzero / len(score_ids),
            "mean_absolute_difference_cp": statistics.fmean(absolute_differences),
            "material": distribution(material_values),
            "fixed_nnue": distribution(nnue_values),
        },
        "search_cost_on_shared_ok": {
            "material": material_cost,
            "fixed_nnue": nnue_cost,
            "fixed_nnue_over_material": {
                "nodes_total_ratio": safe_ratio(
                    float(nnue_cost["nodes_total"]), float(material_cost["nodes_total"])
                ),
                "search_time_total_ratio": safe_ratio(
                    float(nnue_cost["search_time_ms_total"]),
                    float(material_cost["search_time_ms_total"]),
                ),
                "wall_time_total_ratio": safe_ratio(
                    float(nnue_cost["wall_time_ms_total"]),
                    float(material_cost["wall_time_ms_total"]),
                ),
            },
        },
        "definitions": {
            "variance": "population variance over shared successful centipawn records",
            "sign_mismatch": "sign categories -1, 0, +1 differ; zero is its own category",
            "opposite_nonzero_sign": "strict positive-versus-negative disagreement",
            "cost_population": "shared records where both runs completed successfully",
            "multipv_cost": "first line only because aggregate counters repeat per MultiPV line",
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--material", type=Path, required=True)
    parser.add_argument("--fixed-nnue", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()

    try:
        report = compare(args.material, args.fixed_nnue)
    except (OSError, ValueError) as error:
        parser.error(str(error))
    rendered = json.dumps(report, indent=2, ensure_ascii=False, sort_keys=True) + "\n"
    if args.output is None:
        print(rendered, end="")
    else:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered, encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
