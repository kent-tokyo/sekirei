#!/usr/bin/env python3
"""Compare two exact-completed core diagnostic runs under one fixed corpus."""
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
    rows: dict[tuple[str, int], dict] = {}
    for index, item in enumerate(document.get("results", [])):
        source = item.get("source", {})
        key = item.get("id") or (str(source.get("game_id", "")), int(source.get("ply", index)))
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
    left_mean, right_mean = statistics.fmean(left), statistics.fmean(right)
    left_variance = sum((value - left_mean) ** 2 for value in left)
    right_variance = sum((value - right_mean) ** 2 for value in right)
    if left_variance == 0 or right_variance == 0:
        return None
    return sum((a - left_mean) * (b - right_mean) for a, b in zip(left, right, strict=True)) / math.sqrt(left_variance * right_variance)


def load_forcing_classes(path: Path | None) -> dict[str, str]:
    if path is None:
        return {}
    document = json.loads(path.read_text(encoding="utf-8"))
    if document.get("schema") != "sekirei.forcing-position-classification.v1":
        raise ValueError(f"{path}: unsupported forcing classification")
    classes: dict[str, str] = {}
    for entry in document.get("entries", []):
        identifier = entry.get("id")
        forcing_class = entry.get("forcing_class")
        if not isinstance(identifier, str) or forcing_class not in {"forced_defense", "forcing_attack", "quiet"}:
            raise ValueError(f"{path}: invalid forcing classification entry")
        if identifier in classes:
            raise ValueError(f"{path}: duplicate forcing classification id {identifier}")
        classes[identifier] = forcing_class
    return classes


def compare(
    left_path: Path,
    right_path: Path,
    left_label: str,
    right_label: str,
    forcing_classes: dict[str, str] | None = None,
) -> dict:
    left_document, left = load(left_path)
    right_document, right = load(right_path)
    if set(left) != set(right):
        raise ValueError("diagnostic source sets differ")
    left_execution, right_execution = left_document.get("execution"), right_document.get("execution")
    if isinstance(left_execution, dict) and isinstance(right_execution, dict):
        for artifact_name in ("binary", "weights"):
            left_artifact, right_artifact = left_execution.get(artifact_name), right_execution.get(artifact_name)
            if not isinstance(left_artifact, dict) or not isinstance(right_artifact, dict):
                raise ValueError(f"diagnostic execution lacks {artifact_name} provenance")
            if left_artifact.get("sha256") != right_artifact.get("sha256"):
                raise ValueError(f"diagnostic execution differs outside comparison target: {artifact_name}")
        if left_execution.get("corpus_sha256") != right_execution.get("corpus_sha256"):
            raise ValueError("diagnostic execution differs outside comparison target: corpus")
        left_options, right_options = left_execution.get("options"), right_execution.get("options")
        if isinstance(left_options, dict) and isinstance(right_options, dict):
            for key in ("threads", "spec_top_n", "use_book", "tt_mode", "nnue_output"):
                if left_options.get(key) != right_options.get(key):
                    raise ValueError(f"diagnostic execution differs outside comparison target: {key}")
    if left_document.get("nodes") != right_document.get("nodes"):
        raise ValueError("diagnostic node budgets differ")

    forcing_classes = forcing_classes or {}
    rows = []
    for key in sorted(left):
        left_result = left[key]["unrestricted"]
        right_result = right[key]["unrestricted"]
        comparable = completed(left[key]) and completed(right[key])
        source = left[key].get("source", {})
        position_id = key if isinstance(key, str) else f"{key[0]}-ply{key[1]:03}"
        diagnostic = left[key].get("diagnostic", {})
        category = diagnostic.get("forcing_class", left[key].get("source", {}).get("category", "unclassified"))
        rows.append({
            "id": key if isinstance(key, str) else None,
            "game_id": source.get("game_id") if isinstance(source, dict) else key[0],
            "ply": source.get("ply") if isinstance(source, dict) else key[1],
            "category": str(category),
            "forcing_class": forcing_classes.get(position_id, "unclassified"),
            "comparable": comparable,
            "left_score_cp": left_result.get("score_cp") if comparable else None,
            "right_score_cp": right_result.get("score_cp") if comparable else None,
            "left_score_kind": score_kind(left_result.get("score_cp")) if comparable else "missing",
            "right_score_kind": score_kind(right_result.get("score_cp")) if comparable else "missing",
            "left_depth": left_result.get("depth") if comparable else None,
            "right_depth": right_result.get("depth") if comparable else None,
            "same_completed_depth": (
                left_result.get("depth") == right_result.get("depth") if comparable else None
            ),
            "score_delta_right_minus_left_cp": (
                right_result.get("score_cp") - left_result.get("score_cp") if comparable else None
            ),
            "bestmove_changed": left_result.get("bestmove") != right_result.get("bestmove") if comparable else None,
            "left_elapsed_ms": left_result.get("elapsed_ms") if comparable else None,
            "right_elapsed_ms": right_result.get("elapsed_ms") if comparable else None,
        })
    comparable_rows = [row for row in rows if row["comparable"]]
    cp_rows = [row for row in comparable_rows if row["left_score_kind"] == "cp" and row["right_score_kind"] == "cp"]
    left_scores = [float(row["left_score_cp"]) for row in cp_rows]
    right_scores = [float(row["right_score_cp"]) for row in cp_rows]
    deltas = [float(row["score_delta_right_minus_left_cp"]) for row in cp_rows]
    ratios = [
        float(row["right_elapsed_ms"]) / float(row["left_elapsed_ms"])
        for row in cp_rows
        if row["left_elapsed_ms"] and row["right_elapsed_ms"] is not None
    ]
    def summarize(group: list[dict]) -> dict:
        group = [row for row in group if row["left_score_kind"] == "cp" and row["right_score_kind"] == "cp"]
        left_group = [float(row["left_score_cp"]) for row in group]
        right_group = [float(row["right_score_cp"]) for row in group]
        delta_group = [float(row["score_delta_right_minus_left_cp"]) for row in group]
        ratio_group = [
            float(row["right_elapsed_ms"]) / float(row["left_elapsed_ms"])
            for row in group
            if row["left_elapsed_ms"] and row["right_elapsed_ms"] is not None
        ]
        return {
            "cp_comparable": len(group),
            "same_completed_depth": sum(row["same_completed_depth"] is True for row in group),
            "bestmove_changes": sum(row["bestmove_changed"] is True for row in group),
            "sign_mismatches": sum((a > 0) != (b > 0) for a, b in zip(left_group, right_group, strict=True)),
            "score_delta_right_minus_left_mean_cp": statistics.fmean(delta_group) if delta_group else None,
            "score_delta_right_minus_left_mean_abs_cp": statistics.fmean(abs(value) for value in delta_group) if delta_group else None,
            "score_pearson": pearson(left_group, right_group),
            "elapsed_ratio_right_over_left_mean": statistics.fmean(ratio_group) if ratio_group else None,
        }

    overall = summarize(cp_rows)
    return {
        "schema": "sekirei.core-search-config-comparison.v1",
        "diagnostic_only": True,
        "inputs": {
            "left": str(left_path), "right": str(right_path),
            "left_label": left_label, "right_label": right_label,
            "left_execution": left_document.get("execution"),
            "right_execution": right_document.get("execution"),
        },
        "rows": rows,
        "summary": {
            "total": len(rows), "comparable": len(comparable_rows), "cp_comparable": len(cp_rows),
            "same_completed_depth": sum(row["same_completed_depth"] is True for row in cp_rows),
            "mate_or_non_cp": len(comparable_rows) - len(cp_rows),
            "incomplete": len(rows) - len(comparable_rows),
            **overall,
            "score_delta_right_minus_left_stdev_cp": statistics.pstdev(deltas) if len(deltas) > 1 else 0.0 if deltas else None,
        },
        "stratified_summary": {
            category: summarize([row for row in comparable_rows if row["category"] == category])
            for category in sorted({row["category"] for row in comparable_rows})
        },
        "forcing_stratified_summary": {
            forcing_class: summarize([row for row in comparable_rows if row["forcing_class"] == forcing_class])
            for forcing_class in sorted({row["forcing_class"] for row in comparable_rows})
        },
        "claims": {
            "strength": "not_permitted",
            "causal_inference": "not_proven",
            "interpretation": "fixed-corpus diagnostic only; it is not a strength comparison",
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("left", type=Path)
    parser.add_argument("right", type=Path)
    parser.add_argument("--left-label", required=True)
    parser.add_argument("--right-label", required=True)
    parser.add_argument("--forcing-classification", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = compare(
        args.left,
        args.right,
        args.left_label,
        args.right_label,
        load_forcing_classes(args.forcing_classification),
    )
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {result['summary']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
