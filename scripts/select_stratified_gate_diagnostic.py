#!/usr/bin/env python3
"""Select a small, deterministic forcing-class sample from a gate corpus.

The source corpus contains observations, not labels.  This selector only
chooses a bounded review set and preserves that distinction in its output.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

from diagnostic_contract import stable_entry_id


CLASSES = ("forced_defense", "forcing_attack", "quiet")


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def entry_id(entry: dict) -> str:
    return stable_entry_id(entry)


def select(corpus: dict, classification: dict, per_class: int) -> dict:
    if corpus.get("diagnostic_only") is not True or not isinstance(corpus.get("entries"), list):
        raise ValueError("input corpus must be diagnostic-only with entries")
    if classification.get("schema") != "sekirei.forcing-position-classification.v1":
        raise ValueError("unsupported forcing classification")
    entries = corpus["entries"]
    rows = classification.get("entries")
    if not isinstance(rows, list) or len(rows) != len(entries):
        raise ValueError("classification must align one-for-one with corpus entries")
    grouped: dict[str, list[tuple[str, dict]]] = {name: [] for name in CLASSES}
    for index, (entry, row) in enumerate(zip(entries, rows, strict=True)):
        if row.get("id") != entry_id(entry):
            raise ValueError(f"classification id mismatch at index {index}")
        forcing_class = row.get("forcing_class")
        if forcing_class not in grouped:
            raise ValueError(f"unknown forcing class at index {index}")
        grouped[forcing_class].append((entry_id(entry), row))
    for group in grouped.values():
        group.sort(key=lambda item: item[0])
    selected = []
    for forcing_class in CLASSES:
        group = grouped[forcing_class]
        take = min(per_class, len(group))
        for ordinal in range(take):
            identifier, row = group[(ordinal * len(group)) // take]
            entry = next(entry for entry in entries if entry_id(entry) == identifier).copy()
            entry["forcing_class"] = forcing_class
            entry["forcing_features"] = {
                key: row[key] for key in ("in_check", "legal_moves", "legal_captures", "legal_checks")
            }
            selected.append(entry)
    return {
        "schema": "sekirei.stratified-gate-diagnostic-corpus.v1",
        "diagnostic_only": True,
        "strength_claim": "not_permitted",
        "selection_contract": {
            "forcing_classes": list(CLASSES),
            "max_entries_per_class": per_class,
            "strategy": "deterministic_evenly_spaced_stable_id_order",
        },
        "entries": selected,
        "selected_ids": [entry_id(entry) for entry in selected],
        "available": {name: len(grouped[name]) for name in CLASSES},
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--classification", type=Path, required=True)
    parser.add_argument("--per-class", type=int, default=4)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.per_class <= 0:
        parser.error("--per-class must be positive")
    try:
        document = select(
            json.loads(args.corpus.read_text(encoding="utf-8")),
            json.loads(args.classification.read_text(encoding="utf-8")),
            args.per_class,
        )
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        parser.error(str(exc))
    document["inputs"] = {
        "corpus": {"path": str(args.corpus), "sha256": sha256(args.corpus)},
        "classification": {"path": str(args.classification), "sha256": sha256(args.classification)},
    }
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {len(document['entries'])} entries")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
