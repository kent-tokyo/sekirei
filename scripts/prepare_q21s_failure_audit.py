#!/usr/bin/env python3
"""Freeze one blinded diagnostic decision from every Q21r match game."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

import prepare_q21j_failure_audit as base


SCHEMA = "sekirei.q21s-failure-audit-corpus.v1"


def prepare(transcript: Path, result_records: Path, kifu_dir: Path) -> dict:
    document = base.prepare(transcript, result_records, kifu_dir)
    document["schema"] = SCHEMA
    document["phase"] = "Q21s"
    document["source_match"] = "Q21r"
    document["selection_blinding"] = (
        "saved Q21p-candidate online scores and game result only; "
        "no Q21s re-search output"
    )
    for position in document["positions"]:
        position["id"] = position["id"].replace("q21j-game-", "q21s-game-", 1)
    return document


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--transcript", type=Path, required=True)
    parser.add_argument("--result-records", type=Path, required=True)
    parser.add_argument("--kifu-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        document = prepare(args.transcript, args.result_records, args.kifu_dir)
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    print(json.dumps(document["counts"], sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
