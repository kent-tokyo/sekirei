#!/usr/bin/env python3
"""Split diagnostic positions by game/source unit, never by individual rows."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


SCHEMA = "sekirei.floodgate-diagnostic-corpus-split.v1"


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def split(corpus: dict, corpus_hash: str) -> dict:
    entries = corpus.get("entries")
    if not isinstance(entries, list) or not entries:
        raise ValueError("corpus entries must be a non-empty list")
    groups: dict[str, list[int]] = {}
    for index, entry in enumerate(entries):
        game_id = entry.get("source", {}).get("game_id")
        if not isinstance(game_id, str) or not game_id:
            raise ValueError(f"entry {index} lacks source.game_id")
        groups.setdefault(game_id, []).append(index)
    if len(groups) < 2:
        raise ValueError("at least two source games are required for a hold-out split")
    ordered = sorted(groups)
    tuning_games = ordered[: len(ordered) // 2]
    holdout_games = ordered[len(ordered) // 2 :]
    tuning = [i for game in tuning_games for i in groups[game]]
    holdout = [i for game in holdout_games for i in groups[game]]
    return {
        "schema": SCHEMA,
        "diagnostic_only": True,
        "source_schema": corpus.get("schema"),
        "source_corpus_sha256": corpus_hash,
        "rule": "sorted source.game_id groups; whole games only",
        "tuning_entry_indices": tuning,
        "holdout_entry_indices": holdout,
        "tuning_games": tuning_games,
        "holdout_games": holdout_games,
        "claims": {"strength": "not_permitted", "independence": "source-game separation only"},
    }


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("corpus", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    document = json.loads(args.corpus.read_text(encoding="utf-8"))
    result = split(document, sha256(args.corpus))
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: tuning={len(result['tuning_entry_indices'])}, holdout={len(result['holdout_entry_indices'])}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
