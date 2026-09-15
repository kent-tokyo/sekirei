#!/usr/bin/env python3
"""Validate a game-disjoint Floodgate diagnostic tuning/hold-out split."""
from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path


SCHEMA = "sekirei.floodgate-diagnostic-corpus-split.v1"


def validate_documents(corpus: dict, split: dict, corpus_hash: str) -> None:
    if split.get("schema") != SCHEMA or split.get("diagnostic_only") is not True:
        raise ValueError("unexpected split schema")
    if split.get("source_corpus_sha256") != corpus_hash:
        raise ValueError("source corpus hash mismatch")
    entries = corpus.get("entries")
    tuning = split.get("tuning_entry_indices")
    holdout = split.get("holdout_entry_indices")
    if not isinstance(entries, list) or not isinstance(tuning, list) or not isinstance(holdout, list):
        raise ValueError("entries and split indices must be lists")
    if not tuning or not holdout or set(tuning) & set(holdout):
        raise ValueError("tuning and holdout must be non-empty and disjoint")
    if set(tuning) | set(holdout) != set(range(len(entries))):
        raise ValueError("split indices do not cover corpus")
    def games(indices):
        result = set()
        for index in indices:
            if not isinstance(index, int) or index < 0 or index >= len(entries):
                raise ValueError("split index out of range")
            game_id = entries[index].get("source", {}).get("game_id")
            if not isinstance(game_id, str) or not game_id:
                raise ValueError("entry lacks source.game_id")
            result.add(game_id)
        return result
    tuning_games = games(tuning)
    holdout_games = games(holdout)
    if tuning_games & holdout_games:
        raise ValueError("source games overlap between tuning and holdout")
    if set(split.get("tuning_games", [])) != tuning_games or set(split.get("holdout_games", [])) != holdout_games:
        raise ValueError("game manifest does not match indices")


def validate(corpus_path: Path, split_path: Path) -> None:
    corpus_bytes = corpus_path.read_bytes()
    validate_documents(
        json.loads(corpus_bytes),
        json.loads(split_path.read_text(encoding="utf-8")),
        hashlib.sha256(corpus_bytes).hexdigest(),
    )


def main(argv=None) -> int:
    args = argv or sys.argv[1:]
    if len(args) != 2:
        print(f"usage: {Path(sys.argv[0]).name} CORPUS.json SPLIT.json", file=sys.stderr)
        return 2
    try:
        validate(Path(args[0]), Path(args[1]))
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        print(f"invalid diagnostic split: {exc}", file=sys.stderr)
        return 1
    print("diagnostic corpus split OK: game-disjoint tuning/holdout")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
