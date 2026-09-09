#!/usr/bin/env python3
"""Run the same bounded search probe with the built-in material evaluator."""

import argparse
import json
from pathlib import Path

from run_candidate_teacher_probe import load_corpus, run_one, sha256


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--limit", type=int, default=16)
    parser.add_argument("--depth", type=int, default=2)
    parser.add_argument("--timeout", type=float, default=20.0)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.limit <= 0 or args.depth <= 0 or args.timeout <= 0:
        parser.error("limit, depth, and timeout must be positive")
    corpus = load_corpus(args.corpus, args.limit)
    records = [run_one(args.engine, None, row, args.depth, args.timeout) for row in corpus]
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        "\n".join(json.dumps(record, ensure_ascii=False) for record in records) + "\n",
        encoding="utf-8",
    )
    manifest = {
        "schema_version": 1,
        "corpus_sha256": sha256(args.corpus),
        "positions": len(corpus),
        "depth": args.depth,
        "timeout_s": args.timeout,
        "threads": 1,
        "parallel": 1,
        "spec_top_n": 0,
        "evaluator": "material",
        "strength_status": "UNMEASURED",
    }
    print(json.dumps(manifest, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
