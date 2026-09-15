#!/usr/bin/env python3
"""Validate the completed-diagnostic selection boundary."""
from __future__ import annotations

import json
import sys
from pathlib import Path

COMPARABLE_CLASSES = {"root_score_gap_observed", "tt_move_change_observed", "no_difference_observed"}


def validate(document: dict) -> list[str]:
    errors = []
    if document.get("schema") != "sekirei.floodgate-completed-diagnostic-selection.v1":
        errors.append("schema")
    if document.get("diagnostic_only") is not True:
        errors.append("diagnostic_only")
    if document.get("status") not in {"ready", "not_evaluable"}:
        errors.append("status")
    claims = document.get("claims")
    if not isinstance(claims, dict) or claims.get("strength") != "not_permitted":
        errors.append("claims.strength")
    if not isinstance(document.get("rows"), list) or not isinstance(document.get("excluded"), list):
        errors.append("rows/excluded")
    for index, row in enumerate(document.get("rows", [])):
        prefix = f"rows[{index}]"
        if not isinstance(row, dict):
            errors.append(prefix)
            continue
        if row.get("diagnostic_class") not in COMPARABLE_CLASSES:
            errors.append(f"{prefix}.diagnostic_class")
        if row.get("unrestricted_aborted") is not False or row.get("actual_root_aborted") is not False:
            errors.append(f"{prefix}.aborted")
        if row.get("unrestricted_bound") != "exact" or row.get("actual_root_bound") != "exact":
            errors.append(f"{prefix}.bound")
    for index, item in enumerate(document.get("excluded", [])):
        prefix = f"excluded[{index}]"
        if not isinstance(item, dict) or not isinstance(item.get("reasons"), list) or not item["reasons"]:
            errors.append(f"{prefix}.reasons")
    counts = document.get("counts")
    if not isinstance(counts, dict) or counts.get("selected") != len(document.get("rows", [])) \
            or counts.get("excluded") != len(document.get("excluded", [])):
        errors.append("counts")
    if document.get("status") == "ready" and not document.get("rows"):
        errors.append("ready_without_rows")
    if document.get("status") == "not_evaluable" and document.get("rows"):
        errors.append("not_evaluable_with_rows")
    return errors


def main(argv=None) -> int:
    args = argv or sys.argv[1:]
    if len(args) != 1:
        print(f"usage: {Path(sys.argv[0]).name} SELECTION.json", file=sys.stderr)
        return 2
    errors = validate(json.loads(Path(args[0]).read_text(encoding="utf-8")))
    if errors:
        print("invalid completed diagnostic selection: " + ", ".join(errors), file=sys.stderr)
        return 1
    print("completed diagnostic selection valid")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
