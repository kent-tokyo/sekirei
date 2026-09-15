#!/usr/bin/env python3
"""Validate that a selected diagnostic hypothesis matches its source comparison."""
from __future__ import annotations

import argparse
import json
from pathlib import Path

from select_floodgate_hypothesis import select
from validate_floodgate_hypothesis_selection import validate as validate_selection


SUMMARY_KEYS = ("total", "comparable", "nonzero_score_changes")
COMPARISON_SCHEMA = "sekirei.floodgate-budget-comparison.v1"


def validate(comparison: dict, selection: dict) -> list[str]:
    errors = []
    if comparison.get("schema") != COMPARISON_SCHEMA:
        errors.append("comparison.schema")
    summary = comparison.get("summary")
    source_summary = selection.get("source_summary")
    if not isinstance(summary, dict):
        errors.append("comparison.summary")
    if not isinstance(source_summary, dict):
        errors.append("selection.source_summary")
    if isinstance(summary, dict) and isinstance(source_summary, dict):
        for key in SUMMARY_KEYS:
            if source_summary.get(key) != summary.get(key):
                errors.append(f"source_summary.{key}")
    errors.extend(f"selection.{error}" for error in validate_selection(selection))
    try:
        expected = select(comparison)
    except (KeyError, TypeError, ValueError):
        errors.append("comparison.selection_input")
    else:
        if selection.get("selected", {}).get("id") != expected["selected"]["id"]:
            errors.append("selected.id")
    return sorted(set(errors))


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("comparison", type=Path)
    parser.add_argument("selection", type=Path)
    args = parser.parse_args(argv)
    comparison = json.loads(args.comparison.read_text(encoding="utf-8"))
    selection = json.loads(args.selection.read_text(encoding="utf-8"))
    errors = validate(comparison, selection)
    if errors:
        print("inconsistent hypothesis selection: " + ", ".join(errors))
        return 1
    print(f"consistent hypothesis selection: {args.selection}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
