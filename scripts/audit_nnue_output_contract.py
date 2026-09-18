#!/usr/bin/env python3
"""Audit whether an NNUE's representable CP range fits its teacher labels.

This is a diagnostic gate.  It reads the stable ``SEKIRW01`` layout directly,
computes a conservative architectural output bound from the clipped L2 layer,
and compares it with the labels actually supplied to a training run.  It does
not decide playing strength or modify weights.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import statistics
import struct
from pathlib import Path


INPUT = 2420
L1 = 256
L2 = 32
MAGIC = b"SEKIRW01"
CLIPPED_RELU_MAX = 127.0
SCORE_DIVISOR = 64.0


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def output_layer(weights: Path) -> tuple[list[float], float]:
    """Return the output weights and bias from a complete SEKIRW01 file."""
    data = weights.read_bytes()
    ft_bytes = INPUT * L1 * 2
    ft_bias_bytes = L1 * 2
    l2_bytes = 2 * L1 * L2 * 4
    l2_bias_bytes = L2 * 4
    out_offset = len(MAGIC) + ft_bytes + ft_bias_bytes + l2_bytes + l2_bias_bytes
    expected = out_offset + (L2 + 1) * 4
    if len(data) != expected:
        raise ValueError(f"unexpected SEKIRW01 length: got {len(data)}, expected {expected}")
    if data[: len(MAGIC)] != MAGIC:
        raise ValueError("expected SEKIRW01 weights")
    values = struct.unpack_from(f"<{L2 + 1}f", data, out_offset)
    if not all(math.isfinite(value) for value in values):
        raise ValueError("output layer contains a non-finite value")
    return list(values[:-1]), values[-1]


def theoretical_cp_bounds(out: list[float], bias: float) -> tuple[float, float]:
    """Bound score when every L2 activation independently lies in [0, 127]."""
    lower_raw = bias + sum(min(0.0, weight) * CLIPPED_RELU_MAX for weight in out)
    upper_raw = bias + sum(max(0.0, weight) * CLIPPED_RELU_MAX for weight in out)
    return lower_raw / SCORE_DIVISOR, upper_raw / SCORE_DIVISOR


def read_labels(cache: Path) -> list[float]:
    labels: list[float] = []
    for line_number, line in enumerate(cache.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip():
            continue
        row = json.loads(line)
        value = row.get("score_cp")
        if not isinstance(value, (int, float)) or not math.isfinite(value):
            raise ValueError(f"{cache}:{line_number}: finite score_cp is required")
        labels.append(float(value))
    if not labels:
        raise ValueError(f"{cache}: no teacher labels")
    return labels


def percentile(values: list[float], fraction: float) -> float:
    return sorted(values)[round((len(values) - 1) * fraction)]


def label_summary(labels: list[float], cap: float) -> dict:
    capped = [max(-cap, min(cap, value)) for value in labels]
    lower, upper = min(capped), max(capped)
    return {
        "count": len(labels),
        "raw_min_cp": min(labels),
        "raw_median_cp": statistics.median(labels),
        "raw_p95_cp": percentile(labels, 0.95),
        "raw_max_cp": max(labels),
        "cap_cp": cap,
        "at_or_beyond_cap": sum(abs(value) >= cap for value in labels),
        "at_or_beyond_cap_fraction": sum(abs(value) >= cap for value in labels) / len(labels),
        "capped_min_cp": lower,
        "capped_median_cp": statistics.median(capped),
        "capped_std_cp": statistics.pstdev(capped),
        "capped_max_cp": upper,
    }


def audit(weights: Path, cache: Path, cap: float) -> dict:
    out, bias = output_layer(weights)
    lower, upper = theoretical_cp_bounds(out, bias)
    labels = label_summary(read_labels(cache), cap)
    representable = lower <= labels["capped_min_cp"] and upper >= labels["capped_max_cp"]
    return {
        "schema": "sekirei.nnue-output-contract-audit.v1",
        "diagnostic_only": True,
        "strength_claim": False,
        "weights": {"path": str(weights), "sha256": sha256(weights)},
        "teacher_cache": {"path": str(cache), "sha256": sha256(cache)},
        "model": {
            "l2_clipped_relu_range": [0.0, CLIPPED_RELU_MAX],
            "score_divisor": SCORE_DIVISOR,
            "out_bias": bias,
            "negative_output_weights": sum(weight < 0.0 for weight in out),
            "nonnegative_output_weights": sum(weight >= 0.0 for weight in out),
            "theoretical_min_cp": lower,
            "theoretical_max_cp": upper,
            "theoretical_range_cp": upper - lower,
        },
        "labels": labels,
        "conclusion": {
            "all_capped_labels_representable": representable,
            "lower_shortfall_cp": max(0.0, lower - labels["capped_min_cp"]),
            "upper_shortfall_cp": max(0.0, labels["capped_max_cp"] - upper),
            "action": "eligible_for_further_diagnostics" if representable else "block_candidate_from_strength_gate",
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--weights", type=Path, required=True)
    parser.add_argument("--teacher-cache", type=Path, required=True)
    parser.add_argument("--teacher-score-cap", type=float, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if not math.isfinite(args.teacher_score_cap) or args.teacher_score_cap <= 0:
        parser.error("--teacher-score-cap must be finite and positive")
    document = audit(args.weights, args.teacher_cache, args.teacher_score_cap)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(document["conclusion"], sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
