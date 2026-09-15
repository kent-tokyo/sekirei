#!/usr/bin/env python3
"""Summarize forcing classes by verified color-reversed gate-pair outcome."""
from __future__ import annotations

import argparse
import json
from collections import Counter, defaultdict
from pathlib import Path

from diagnostic_contract import stable_entry_id


def diagnostic_id(entry: dict) -> str:
    return stable_entry_id(entry)


def rows(corpus: dict, classification: dict, pairs: dict) -> list[dict]:
    if corpus.get("diagnostic_only") is not True or classification.get("diagnostic_only") is not True:
        raise ValueError("inputs must be diagnostic-only")
    entries, classes = corpus.get("entries"), classification.get("entries")
    if not isinstance(entries, list) or not isinstance(classes, list) or len(entries) != len(classes):
        raise ValueError("corpus and classification must align")
    pair_class = {row.get("pair_id"): row.get("pair_class") for row in pairs.get("pairs", [])}
    result = []
    for entry, cls in zip(entries, classes, strict=True):
        if cls.get("id") != diagnostic_id(entry):
            raise ValueError("classification id mismatch")
        source = entry.get("source", {})
        pair_id = source.get("pair_id")
        if pair_id not in pair_class:
            raise ValueError(f"unknown pair_id {pair_id!r}")
        forcing_class = cls.get("forcing_class")
        if forcing_class not in {"forced_defense", "forcing_attack", "quiet"}:
            raise ValueError("invalid forcing class")
        result.append({
            "pair_id": pair_id,
            "pair_class": pair_class[pair_id],
            "game_outcome": source.get("outcome"),
            "forcing_class": forcing_class,
            "selection_reason": entry.get("selection_reason"),
        })
    return result


def summarize(rows_: list[dict]) -> dict:
    grouped: dict[str, Counter] = defaultdict(Counter)
    outcomes: dict[str, Counter] = defaultdict(Counter)
    for row in rows_:
        grouped[row["pair_class"]][row["forcing_class"]] += 1
        outcomes[row["pair_class"]][row["game_outcome"]] += 1
    return {
        pair_class: {
            "positions": sum(counts.values()),
            "forcing_classes": dict(sorted(counts.items())),
            "game_outcomes": dict(sorted(outcomes[pair_class].items())),
        }
        for pair_class, counts in sorted(grouped.items())
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pair-outcomes", type=Path, required=True)
    parser.add_argument("--corpus", type=Path, action="append", required=True)
    parser.add_argument("--classification", type=Path, action="append", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if len(args.corpus) != len(args.classification):
        parser.error("--corpus and --classification counts must match")
    try:
        pairs = json.loads(args.pair_outcomes.read_text(encoding="utf-8"))
        all_rows = []
        for corpus_path, class_path in zip(args.corpus, args.classification, strict=True):
            all_rows.extend(rows(
                json.loads(corpus_path.read_text(encoding="utf-8")),
                json.loads(class_path.read_text(encoding="utf-8")),
                pairs,
            ))
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        parser.error(str(exc))
    document = {
        "schema": "sekirei.gate-pair-class-diagnostic.v1",
        "diagnostic_only": True,
        "strength_claim": "not_permitted",
        "rows": all_rows,
        "summary": summarize(all_rows),
    }
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {len(all_rows)} diagnostic rows")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
