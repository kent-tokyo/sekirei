#!/usr/bin/env python3
"""Summarize baseline/candidate root-profile errors by fixed strata.

Each input must be a `sekirei.nnue-root-profile-comparison.v1` document with
the same baseline and teacher settings.  This is a calibration diagnostic; it
does not compare playing strength or select a release candidate.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import statistics
from collections import defaultdict
from pathlib import Path
from typing import Any


MATE_THRESHOLD = 899_000


def absolute_error(score: int, teacher: int) -> int:
    return abs(score - teacher)


def same_sign(score: int, teacher: int) -> bool:
    return (score >= 0) == (teacher >= 0)


def strata(row: dict[str, Any], labels: dict[str, int]) -> dict[str, str]:
    attributes = row["attributes"]
    # Prefer the frozen source label.  The profile's teacher score is a fresh
    # re-search and may legitimately differ in mate detection at another
    # node budget.
    teacher = labels.get(row.get("sfen", ""), row["teacher"]["score_cp"])
    return {
        "all": "all",
        "phase": attributes["phase"],
        "material_band": attributes["material_band"],
        "teacher_class": "mate" if abs(teacher) >= MATE_THRESHOLD else "non_mate",
    }


def summarize_rows(
    rows: list[dict[str, Any]], labels: dict[str, int] | None = None
) -> dict[str, dict[str, dict[str, float | int]]]:
    labels = labels or {}
    buckets: dict[str, dict[str, list[dict[str, Any]]]] = defaultdict(lambda: defaultdict(list))
    for row in rows:
        if not row.get("comparable") or not isinstance(row.get("teacher"), dict):
            continue
        for axis, value in strata(row, labels).items():
            buckets[axis][value].append(row)
    result: dict[str, dict[str, dict[str, float | int]]] = {}
    for axis, groups in buckets.items():
        result[axis] = {}
        for value, group in sorted(groups.items()):
            baseline_errors = [absolute_error(row["baseline"]["score_cp"], row["teacher"]["score_cp"]) for row in group]
            candidate_errors = [absolute_error(row["candidate"]["score_cp"], row["teacher"]["score_cp"]) for row in group]
            result[axis][value] = {
                "n": len(group),
                "baseline_abs_error_mean_cp": statistics.fmean(baseline_errors),
                "candidate_abs_error_mean_cp": statistics.fmean(candidate_errors),
                "candidate_minus_baseline_abs_error_mean_cp": statistics.fmean(candidate_errors) - statistics.fmean(baseline_errors),
                "baseline_same_sign": sum(same_sign(row["baseline"]["score_cp"], row["teacher"]["score_cp"]) for row in group),
                "candidate_same_sign": sum(same_sign(row["candidate"]["score_cp"], row["teacher"]["score_cp"]) for row in group),
                "bestmove_changed": sum(row["bestmove_changed"] for row in group),
            }
    return result


def read_profile(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if value.get("schema") != "sekirei.nnue-root-profile-comparison.v1":
        raise ValueError(f"unsupported profile schema: {path}")
    if not isinstance(value.get("rows"), list):
        raise ValueError(f"profile has no rows: {path}")
    return value


def read_teacher_label_manifest(path: Path) -> tuple[dict[str, int], str]:
    raw = path.read_bytes()
    value = json.loads(raw)
    if value.get("schema") != "sekirei.teacher-strata-corpus.v1":
        raise ValueError(f"unsupported teacher label manifest: {path}")
    rows = value.get("rows")
    if not isinstance(rows, list):
        raise ValueError(f"teacher label manifest has no rows: {path}")
    labels: dict[str, int] = {}
    for index, row in enumerate(rows, 1):
        if not isinstance(row, dict):
            raise ValueError(f"teacher label manifest row {index} is not an object")
        sfen, score = row.get("sfen"), row.get("teacher_score_cp")
        if not isinstance(sfen, str) or not isinstance(score, int):
            raise ValueError(f"teacher label manifest row {index} lacks sfen or score")
        if sfen in labels and labels[sfen] != score:
            raise ValueError(f"teacher label manifest has conflicting score for {sfen}")
        labels[sfen] = score
    return labels, hashlib.sha256(raw).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--profile", action="append", type=Path, required=True)
    parser.add_argument(
        "--teacher-label-manifest",
        type=Path,
        help="optional frozen label manifest; uses its mate/non-mate class instead of re-search score",
    )
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    labels: dict[str, int] = {}
    label_input: dict[str, str] | None = None
    if args.teacher_label_manifest:
        labels, digest = read_teacher_label_manifest(args.teacher_label_manifest)
        label_input = {"path": str(args.teacher_label_manifest), "sha256": digest}
    profiles = []
    for path in args.profile:
        value = read_profile(path)
        profiles.append({"path": str(path), "summary": summarize_rows(value["rows"], labels)})
    output = {
        "schema": "sekirei.nnue-root-profile-strata.v1",
        "diagnostic_only": True,
        "strength_claim": False,
        "mate_threshold_cp": MATE_THRESHOLD,
        "teacher_label_manifest": label_input,
        "profiles": profiles,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(output, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"profiles": len(profiles)}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
