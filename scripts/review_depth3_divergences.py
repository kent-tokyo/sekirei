#!/usr/bin/env python3
"""Review depth-3 candidate/teacher move divergences against material."""

import argparse
import json
from collections import Counter
from pathlib import Path


def load(path: Path) -> dict[str, dict]:
    return {json.loads(line)["sample_id"]: json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()}


def score(row: dict) -> float:
    return float(row["lines"][0]["score_cp"])


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("candidate", type=Path)
    parser.add_argument("teacher", type=Path)
    parser.add_argument("material", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    candidate, teacher, material = (load(path) for path in (args.candidate, args.teacher, args.material))
    rows = []
    for sample_id in sorted(candidate.keys() & teacher.keys() & material.keys()):
        c, t, m = candidate[sample_id], teacher[sample_id], material[sample_id]
        if all(row.get("status") == "ok" for row in (c, t, m)) and c["bestmove"] != t["bestmove"]:
            c_move, t_move, m_move = c["bestmove"], t["bestmove"], m["bestmove"]
            if c_move == m_move:
                bucket = "candidate_material_agree"
            elif t_move == m_move:
                bucket = "teacher_material_agree"
            else:
                bucket = "all_three_differ"
            rows.append({
                "sample_id": sample_id,
                "candidate_move": c_move,
                "teacher_move": t_move,
                "material_move": m_move,
                "move_bucket": bucket,
                "candidate_score": score(c),
                "teacher_score": score(t),
                "material_score": score(m),
                "candidate_material_distance": abs(score(c) - score(m)),
                "teacher_material_distance": abs(score(t) - score(m)),
            })
    report = {
        "schema_version": 1,
        "diagnostic_only": True,
        "depth": 3,
        "divergent_positions": len(rows),
        "move_buckets": dict(sorted(Counter(row["move_bucket"] for row in rows).items())),
        "candidate_closer_to_material": sum(row["candidate_material_distance"] < row["teacher_material_distance"] for row in rows),
        "teacher_closer_to_material": sum(row["teacher_material_distance"] < row["candidate_material_distance"] for row in rows),
        "records": rows,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({key: value for key, value in report.items() if key != "records"}, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
