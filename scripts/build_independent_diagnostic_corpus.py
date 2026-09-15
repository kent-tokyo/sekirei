#!/usr/bin/env python3
"""Convert the repository's fixed-depth positions into an unlabeled hold-out corpus."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def build(document: dict, source_hash: str) -> dict:
    positions = document.get("positions")
    if not isinstance(positions, list) or not positions:
        raise ValueError("fixed-depth corpus has no positions")
    entries = []
    excluded = []
    for position in positions:
        if not isinstance(position, dict) or not isinstance(position.get("id"), str):
            raise ValueError("each position requires id")
        if not isinstance(position.get("sfen"), str):
            excluded.append({"id": position["id"], "reason": "SFEN must be materialized by a separate replay step"})
            continue
        entries.append({
            "source": {"game_id": f"fixed-depth:{position['id']}", "source": position.get("source", "unknown")},
            "position": {
                "sfen": position["sfen"],
                "history_before": [],
                "actual_move_observed": None,
            },
            "label_policy": "unlabeled_independent_holdout_no_correct_move_label",
        })
    return {
        "schema": "sekirei.floodgate-diagnostic-corpus.v1",
        "diagnostic_only": True,
        "split": "independent_holdout_only",
        "source_corpus_sha256": source_hash,
        "entries": entries,
        "excluded_positions": excluded,
        "invalid_games": [],
        "strength_claim": "not_permitted",
    }


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("corpus", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    result = build(json.loads(args.corpus.read_text(encoding="utf-8")), sha256(args.corpus))
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {len(result['entries'])} independent holdout positions")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
