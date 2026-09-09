#!/usr/bin/env python3
"""Classify candidate/teacher/material score and move divergences."""

import argparse
import json
from collections import Counter
from pathlib import Path


def load(path: Path) -> dict[str, dict]:
    rows = {}
    for raw in path.read_text(encoding="utf-8").splitlines():
        if raw.strip():
            row = json.loads(raw)
            rows[row["sample_id"]] = row
    return rows


def score(row: dict) -> float:
    return float(row["lines"][0]["score_cp"])


def sign(value: float) -> int:
    return (value > 0) - (value < 0)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("candidate", type=Path)
    parser.add_argument("teacher", type=Path)
    parser.add_argument("material", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    candidate, teacher, material = (load(path) for path in (args.candidate, args.teacher, args.material))
    ids = sorted(candidate.keys() & teacher.keys() & material.keys())
    rows = [
        (candidate[sample_id], teacher[sample_id], material[sample_id])
        for sample_id in ids
        if all(row.get("status") == "ok" for row in (candidate[sample_id], teacher[sample_id], material[sample_id]))
    ]
    move_pairs = {
        "candidate_teacher": sum(c["bestmove"] != t["bestmove"] for c, t, _ in rows),
        "candidate_material": sum(c["bestmove"] != m["bestmove"] for c, _, m in rows),
        "teacher_material": sum(t["bestmove"] != m["bestmove"] for _, t, m in rows),
    }
    closeness = Counter()
    sign_patterns = Counter()
    for candidate_row, teacher_row, material_row in rows:
        c, t, m = score(candidate_row), score(teacher_row), score(material_row)
        distance_c, distance_t = abs(c - m), abs(t - m)
        closeness["candidate_closer"] += distance_c < distance_t
        closeness["teacher_closer"] += distance_t < distance_c
        closeness["equal_distance"] += distance_c == distance_t
        sign_patterns[f"{sign(c)}:{sign(t)}:{sign(m)}"] += 1
    report = {
        "schema_version": 1,
        "diagnostic_only": True,
        "shared_ids": len(ids),
        "shared_ok": len(rows),
        "move_mismatch_counts": move_pairs,
        "score_closeness_to_material": dict(sorted(closeness.items())),
        "score_sign_patterns_candidate_teacher_material": dict(sorted(sign_patterns.items())),
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
