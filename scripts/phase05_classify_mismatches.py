#!/usr/bin/env python3
"""Classify parity reports without making a performance claim.

Input is JSONL with one comparison row per implementation/case/field.  This
keeps a minimized mismatch useful even when an external adapter is unavailable.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path


def classify(field: str, expected: object, actual: object) -> str:
    if field == "sfen":
        return "sfen_format"
    if field in {"legal_moves", "perft"}:
        return "movegen_or_rules"
    if field in {"do_undo", "hash"}:
        return "state_transition"
    if field in {"repetition", "check_detection"}:
        return "rule_resolution"
    if field in {"usi", "option"}:
        return "protocol"
    return "unclassified"


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("report", type=Path, help="JSONL comparison report")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    rows = []
    for number, line in enumerate(args.report.read_text().splitlines(), 1):
        if not line.strip():
            continue
        row = json.loads(line)
        if row.get("match") is False:
            row["line"] = number
            row["classification"] = classify(row.get("field", ""), row.get("expected"), row.get("actual"))
            rows.append(row)
    result = {
        "schema": "sekirei.parity-mismatch.v1",
        "status": "CLEAN" if not rows else "MISMATCHES_FOUND",
        "count": len(rows),
        "mismatches": rows,
    }
    text = json.dumps(result, indent=2, ensure_ascii=False) + "\n"
    if args.output:
        args.output.write_text(text)
    else:
        print(text, end="")


if __name__ == "__main__":
    main()
