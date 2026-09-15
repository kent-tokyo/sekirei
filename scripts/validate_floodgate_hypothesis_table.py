#!/usr/bin/env python3
"""Validate a non-causal diagnostic hypothesis table."""
from __future__ import annotations

import json
import sys
from pathlib import Path


SCHEMA = "sekirei.floodgate-diagnostic-hypotheses.v1"
REQUIRED = ("diagnostic_class", "observed_rows", "status", "hypothesis", "support", "reject", "next_test")


def validate(document: dict) -> list[str]:
    errors = []
    if document.get("schema") != SCHEMA:
        errors.append("schema")
    if document.get("diagnostic_only") is not True:
        errors.append("diagnostic_only")
    claims = document.get("claims")
    if not isinstance(claims, dict):
        errors.append("claims")
    else:
        if claims.get("strength") != "not_permitted":
            errors.append("claims.strength")
        if claims.get("causal_inference") != "not_proven":
            errors.append("claims.causal_inference")
        if claims.get("played_move_is_label") is not False:
            errors.append("claims.played_move_is_label")
    rows = document.get("rows")
    if not isinstance(rows, list) or not rows:
        return errors + ["rows"]
    source = document.get("source")
    if not isinstance(source, dict):
        errors.append("source")
    else:
        for key in ("source_revision", "corpus_sha256"):
            if not isinstance(source.get(key), str) or not source[key]:
                errors.append(f"source.{key}")
        if not isinstance(source.get("binary"), dict):
            errors.append("source.binary")
        if source.get("weights") is not None and not isinstance(source.get("weights"), dict):
            errors.append("source.weights")
        if not isinstance(source.get("options"), dict):
            errors.append("source.options")
    for index, row in enumerate(rows):
        prefix = f"rows[{index}]"
        if not isinstance(row, dict):
            errors.append(prefix)
            continue
        for key in REQUIRED[0:1] + REQUIRED[2:]:
            if not isinstance(row.get(key), str) or not row[key]:
                errors.append(f"{prefix}.{key}")
        if not isinstance(row.get("observed_rows"), int) or row["observed_rows"] < 0:
            errors.append(f"{prefix}.observed_rows")
        if row.get("status") not in {"support", "reject", "unknown"}:
            errors.append(f"{prefix}.status")
    return errors


def main(argv=None) -> int:
    args = argv or sys.argv[1:]
    if len(args) != 1:
        print(f"usage: {Path(sys.argv[0]).name} HYPOTHESES.json", file=sys.stderr)
        return 2
    document = json.loads(Path(args[0]).read_text(encoding="utf-8"))
    errors = validate(document)
    if errors:
        print("invalid hypothesis table: " + ", ".join(errors), file=sys.stderr)
        return 1
    print(f"valid hypothesis table: {args[0]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
