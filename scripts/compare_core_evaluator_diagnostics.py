#!/usr/bin/env python3
"""Compare a fixed evaluator baseline and candidate on shared positions."""
from __future__ import annotations

import argparse
import json
import math
import statistics
from pathlib import Path

from diagnostic_contract import score_kind


SCHEMAS = {"sekirei.floodgate-core-diagnostic.v2", "sekirei.floodgate-core-diagnostic.v3"}


def load(path: Path) -> tuple[dict, dict[tuple[str, int], dict]]:
    document = json.loads(path.read_text(encoding="utf-8"))
    if document.get("schema") not in SCHEMAS or document.get("diagnostic_only") is not True:
        raise ValueError(f"{path}: unsupported diagnostic input")
    rows = {}
    for index, item in enumerate(document.get("results", [])):
        source = item.get("source", {})
        raw_ply = source.get("ply", index)
        key = item.get("id") or (str(source.get("game_id", "")), int(raw_ply))
        valid_key = (
            isinstance(key, str) and bool(key)
        ) or (
            isinstance(key, tuple) and len(key) == 2 and isinstance(key[0], str)
            and bool(key[0]) and isinstance(key[1], int) and key[1] >= 0
        )
        if not valid_key or key in rows:
            raise ValueError(f"{path}: invalid or duplicate source key")
        rows[key] = item
    if not rows:
        raise ValueError(f"{path}: no results")
    return document, rows


def completed(item: dict) -> bool:
    result = item.get("unrestricted", {})
    return (
        result.get("completion") == "search_completed"
        and result.get("completed_iteration_valid") == "true"
        and result.get("completed_bound") == "exact"
    )


def pearson(left: list[float], right: list[float]) -> float | None:
    if len(left) < 2:
        return None
    lm, rm = statistics.fmean(left), statistics.fmean(right)
    lx = sum((value - lm) ** 2 for value in left)
    rx = sum((value - rm) ** 2 for value in right)
    if lx == 0 or rx == 0:
        return None
    return sum((a - lm) * (b - rm) for a, b in zip(left, right, strict=True)) / math.sqrt(lx * rx)


def compare(baseline_path: Path, candidate_path: Path, *, allow_nonmaterial_baseline: bool = False) -> dict:
    baseline_document, baseline = load(baseline_path)
    candidate_document, candidate = load(candidate_path)
    baseline_mode = baseline_document.get("eval_mode", "material")
    candidate_mode = candidate_document.get("eval_mode")
    if not isinstance(baseline_mode, str) or not baseline_mode:
        raise ValueError(f"{baseline_path}: missing baseline eval_mode")
    if not allow_nonmaterial_baseline and baseline_mode != "material":
        raise ValueError(f"{baseline_path}: expected material baseline, got {baseline_mode!r}")
    if not isinstance(candidate_mode, str) or not candidate_mode:
        raise ValueError(f"{candidate_path}: missing candidate eval_mode")
    if set(baseline) != set(candidate):
        raise ValueError("baseline/candidate source sets differ")
    rows = []
    for key in sorted(baseline):
        base, candidate_item = baseline[key], candidate[key]
        base_result, candidate_result = base["unrestricted"], candidate_item["unrestricted"]
        both = completed(base) and completed(candidate_item)
        diagnostic = base.get("diagnostic", {})
        category = diagnostic.get("forcing_class", base.get("source", {}).get("category", "unclassified"))
        source = base.get("source", {})
        rows.append({
            "id": key if isinstance(key, str) else None,
            "game_id": source.get("game_id") if isinstance(source, dict) else key[0],
            "ply": source.get("ply") if isinstance(source, dict) else key[1], "comparable": both,
            "category": str(category),
            "baseline_score_cp": base_result.get("score_cp") if both else None,
            "candidate_score_cp": candidate_result.get("score_cp") if both else None,
            "baseline_score_kind": score_kind(base_result.get("score_cp")) if both else "missing",
            "candidate_score_kind": score_kind(candidate_result.get("score_cp")) if both else "missing",
            "score_delta_candidate_minus_baseline_cp": candidate_result.get("score_cp") - base_result.get("score_cp") if both else None,
            "bestmove_changed": base_result.get("bestmove") != candidate_result.get("bestmove") if both else None,
            "baseline_elapsed_ms": base_result.get("elapsed_ms") if both else None,
            "candidate_elapsed_ms": candidate_result.get("elapsed_ms") if both else None,
            "baseline_depth": base_result.get("depth") if both else None,
            "candidate_depth": candidate_result.get("depth") if both else None,
        })
    comparable = [row for row in rows if row["comparable"]]
    cp_comparable = [row for row in comparable if row["baseline_score_kind"] == "cp" and row["candidate_score_kind"] == "cp"]
    baseline_scores = [float(row["baseline_score_cp"]) for row in cp_comparable]
    candidate_scores = [float(row["candidate_score_cp"]) for row in cp_comparable]
    deltas = [float(row["score_delta_candidate_minus_baseline_cp"]) for row in cp_comparable]
    def summarize(group: list[dict]) -> dict:
        group = [row for row in group if row["baseline_score_kind"] == "cp" and row["candidate_score_kind"] == "cp"]
        deltas = [float(row["score_delta_candidate_minus_baseline_cp"]) for row in group]
        return {
            "cp_comparable": len(group),
            "score_sign_partition": "positive_vs_non_positive",
            "bestmove_changes": sum(row["bestmove_changed"] is True for row in group),
            "sign_mismatches": sum(
                (row["baseline_score_cp"] > 0) != (row["candidate_score_cp"] > 0)
                for row in group
            ),
            "score_delta_mean_cp": statistics.fmean(deltas) if deltas else None,
            "score_pearson": pearson(
                [float(row["baseline_score_cp"]) for row in group],
                [float(row["candidate_score_cp"]) for row in group],
            ),
        }
    categories = {
        category: summarize([row for row in comparable if row["category"] == category])
        for category in sorted({row["category"] for row in comparable})
    }
    return {
        "schema": "sekirei.core-evaluator-comparison.v3", "diagnostic_only": True,
        "inputs": {
            "baseline": str(baseline_path), "baseline_eval_mode": baseline_mode,
            "candidate": str(candidate_path), "candidate_eval_mode": candidate_mode,
        },
        "rows": rows,
        "summary": {
            "total": len(rows), "comparable": len(comparable), "cp_comparable": len(cp_comparable),
            "mate_or_non_cp": len(comparable) - len(cp_comparable), "incomplete": len(rows) - len(comparable),
            "score_sign_partition": "positive_vs_non_positive",
            "bestmove_changes": sum(row["bestmove_changed"] is True for row in cp_comparable),
            "depth_changes": sum(row["baseline_depth"] != row["candidate_depth"] for row in cp_comparable),
            "sign_mismatches": sum((a > 0) != (b > 0) for a, b in zip(baseline_scores, candidate_scores, strict=True)),
            "score_delta_mean_cp": statistics.fmean(deltas) if deltas else None,
            "score_delta_stdev_cp": statistics.pstdev(deltas) if len(deltas) > 1 else 0.0 if deltas else None,
            "score_pearson": pearson(baseline_scores, candidate_scores),
            "elapsed_ratio_candidate_over_baseline_mean": statistics.fmean(
                float(row["candidate_elapsed_ms"]) / float(row["baseline_elapsed_ms"])
                for row in comparable if row["baseline_elapsed_ms"] and row["candidate_elapsed_ms"] is not None
            ) if any(row["baseline_elapsed_ms"] and row["candidate_elapsed_ms"] is not None for row in comparable) else None,
        },
        "stratified_summary": categories,
        "claims": {"strength": "not_permitted", "causal_inference": "not_proven",
                   "interpretation": "evaluator output and cost diagnostic only"},
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("baseline", type=Path)
    parser.add_argument("candidate", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--allow-nonmaterial-baseline", action="store_true")
    args = parser.parse_args()
    result = compare(args.baseline, args.candidate, allow_nonmaterial_baseline=args.allow_nonmaterial_baseline)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {result['summary']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
