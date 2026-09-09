#!/usr/bin/env python3
"""Run a small fixed-depth candidate/teacher sweep on one stable corpus prefix."""

import argparse
import json
from pathlib import Path

from run_candidate_teacher_probe import load_corpus, run_one, sha256


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--teacher", type=Path, required=True)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--limit", type=int, default=64)
    parser.add_argument("--depths", type=int, nargs="+", default=[1, 2, 3])
    parser.add_argument("--timeout", type=float, default=20.0)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    if args.limit <= 0 or any(depth <= 0 for depth in args.depths) or args.timeout <= 0:
        parser.error("limit, depths, and timeout must be positive")
    corpus = load_corpus(args.corpus, args.limit)
    args.output_dir.mkdir(parents=True, exist_ok=True)
    manifests = []
    for depth in args.depths:
        candidate_output = args.output_dir / f"candidate_depth{depth}.jsonl"
        teacher_output = args.output_dir / f"teacher_depth{depth}.jsonl"
        for weight, output in ((args.candidate, candidate_output), (args.teacher, teacher_output)):
            records = [run_one(args.engine, weight, row, depth, args.timeout) for row in corpus]
            output.write_text(
                "\n".join(json.dumps(record, ensure_ascii=False) for record in records) + "\n",
                encoding="utf-8",
            )
        manifests.append({
            "depth": depth,
            "positions": len(corpus),
            "candidate_output": str(candidate_output),
            "teacher_output": str(teacher_output),
        })
    manifest = {
        "schema_version": 1,
        "diagnostic_only": True,
        "corpus_sha256": sha256(args.corpus),
        "limit": len(corpus),
        "threads": 1,
        "parallel": 1,
        "spec_top_n": 0,
        "sweep": manifests,
    }
    manifest_path = args.output_dir / "manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(manifest, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
