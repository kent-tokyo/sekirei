#!/usr/bin/env python3
"""Validate the single-hypothesis diagnostic boundary."""
from __future__ import annotations

import json
import sys
from pathlib import Path


SCHEMA = "sekirei.floodgate-hypothesis-selection.v1"
REQUIRED = ("id", "rationale", "expected_change", "rejection_condition", "next_test")


def validate(document: dict) -> list[str]:
    errors = []
    if document.get("schema") != SCHEMA:
        errors.append("schema")
    if document.get("diagnostic_only") is not True:
        errors.append("diagnostic_only")
    selected = document.get("selected")
    if not isinstance(selected, dict):
        return errors + ["selected"]
    for key in REQUIRED:
        if not isinstance(selected.get(key), str) or not selected[key]:
            errors.append(f"selected.{key}")
    claims = document.get("claims")
    if not isinstance(claims, dict):
        errors.append("claims")
    else:
        if claims.get("strength") != "not_permitted":
            errors.append("claims.strength")
        if claims.get("causal_inference") != "not_proven":
            errors.append("claims.causal_inference")
        if claims.get("implementation_adoption") != "not_yet":
            errors.append("claims.implementation_adoption")
    return errors


def main(argv=None) -> int:
    args = argv or sys.argv[1:]
    if len(args) != 1:
        print(f"usage: {Path(sys.argv[0]).name} SELECTION.json", file=sys.stderr)
        return 2
    document = json.loads(Path(args[0]).read_text(encoding="utf-8"))
    errors = validate(document)
    if errors:
        print("invalid hypothesis selection: " + ", ".join(errors), file=sys.stderr)
        return 1
    print(f"valid hypothesis selection: {args[0]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
