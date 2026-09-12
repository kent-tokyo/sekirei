#!/usr/bin/env python3
"""Create the deterministic tuning/hold-out split for the SP0 corpus."""

import argparse
import hashlib
import json
from pathlib import Path


def cases_hash(cases: list[dict]) -> str:
    canonical = json.dumps(cases, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def split(corpus_path: Path, output_path: Path) -> None:
    corpus = json.loads(corpus_path.read_text(encoding="utf-8"))
    cases = corpus["cases"]
    groups = {}
    for case in cases:
        group_id = case["id"].removesuffix("-opposite")
        groups.setdefault(case["category"], {}).setdefault(group_id, []).append(case["id"])
    tuning, holdout = [], []
    group_manifest = {"tuning": [], "holdout": []}
    for category in sorted(groups):
        category_groups = sorted(groups[category])
        if len(category_groups) != 8:
            raise ValueError(f"{category}: expected 8 base groups")
        for index, group_id in enumerate(category_groups):
            destination = tuning if index < 4 else holdout
            name = "tuning" if index < 4 else "holdout"
            destination.extend(sorted(groups[category][group_id]))
            group_manifest[name].append(f"{category}/{group_id}")
    if len(tuning) != 64 or len(holdout) != 64 or set(tuning) & set(holdout):
        raise ValueError("split must contain 64 disjoint cases per side")
    result = {
        "schema": "sekirei.speed-corpus-split.v1",
        "version": 1,
        "source_schema": corpus["schema"],
        "source_corpus_sha256": corpus["corpus_sha256"],
        "rule": "first four sorted base groups per category are tuning; remaining four are hold-out",
        "tuning_case_ids": tuning,
        "holdout_case_ids": holdout,
        "groups": group_manifest,
    }
    output_path.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    split(args.corpus.resolve(strict=True), args.output)
    print("speed corpus split: tuning=64 holdout=64")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
