#!/usr/bin/env python3
"""Summarize candidate/teacher fixed-depth sweep outputs."""

import argparse
import json
from pathlib import Path

from compare_teacher_evals import compare


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    reports = {}
    for candidate_path in sorted(args.directory.glob("candidate_depth*.jsonl")):
        depth = candidate_path.stem.removeprefix("candidate_depth")
        teacher_path = args.directory / f"teacher_depth{depth}.jsonl"
        report = compare(candidate_path, teacher_path)
        shared = report["coverage"]["shared_ok_records"]
        reports[depth] = {
            "shared_ok": shared,
            "bestmove_mismatch": None,
            "scores": report["scores"],
            "search_cost": report["search_cost_on_shared_ok"],
        }
        candidate_rows = {json.loads(line)["sample_id"]: json.loads(line) for line in candidate_path.read_text(encoding="utf-8").splitlines() if line.strip()}
        teacher_rows = {json.loads(line)["sample_id"]: json.loads(line) for line in teacher_path.read_text(encoding="utf-8").splitlines() if line.strip()}
        ok_ids = [sample_id for sample_id in candidate_rows.keys() & teacher_rows.keys() if candidate_rows[sample_id].get("status") == "ok" and teacher_rows[sample_id].get("status") == "ok"]
        reports[depth]["bestmove_mismatch"] = sum(candidate_rows[sample_id]["bestmove"] != teacher_rows[sample_id]["bestmove"] for sample_id in ok_ids)
        reports[depth]["bestmove_mismatch_rate"] = reports[depth]["bestmove_mismatch"] / len(ok_ids) if ok_ids else None
    output = {"schema_version": 1, "diagnostic_only": True, "depths": reports}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(output, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(json.dumps(output, indent=2, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
