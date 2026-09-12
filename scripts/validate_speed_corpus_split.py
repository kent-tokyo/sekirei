#!/usr/bin/env python3
"""Validate the deterministic SP0 tuning/hold-out split."""

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CORPUS = ROOT / "scripts/fixtures/speed_corpus_v1.json"
SPLIT = ROOT / "scripts/fixtures/speed_corpus_split_v1.json"


def validate(corpus_path: Path = CORPUS, split_path: Path = SPLIT) -> None:
    corpus = json.loads(corpus_path.read_text(encoding="utf-8"))
    split = json.loads(split_path.read_text(encoding="utf-8"))
    if split.get("schema") != "sekirei.speed-corpus-split.v1" or split.get("version") != 1:
        raise ValueError("unexpected split schema")
    actual = {case["id"] for case in corpus["cases"]}
    tuning = split.get("tuning_case_ids")
    holdout = split.get("holdout_case_ids")
    if not isinstance(tuning, list) or not isinstance(holdout, list):
        raise ValueError("split IDs must be lists")
    if len(tuning) != 64 or len(holdout) != 64:
        raise ValueError("split must contain 64 cases per side")
    if len(set(tuning)) != 64 or len(set(holdout)) != 64 or set(tuning) & set(holdout):
        raise ValueError("split IDs must be disjoint and unique")
    if set(tuning) | set(holdout) != actual:
        raise ValueError("split does not cover the corpus")
    if split.get("source_corpus_sha256") != corpus.get("corpus_sha256"):
        raise ValueError("split source hash does not match corpus")
    groups = split.get("groups")
    if not isinstance(groups, dict) or len(groups.get("tuning", [])) != 32 or len(groups.get("holdout", [])) != 32:
        raise ValueError("split group manifest is incomplete")


def main() -> int:
    try:
        validate(Path(sys.argv[1]) if len(sys.argv) > 1 else CORPUS,
                 Path(sys.argv[2]) if len(sys.argv) > 2 else SPLIT)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"speed corpus split invalid: {error}", file=sys.stderr)
        return 1
    print("speed corpus split OK: tuning=64, holdout=64")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
