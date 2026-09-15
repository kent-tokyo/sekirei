#!/usr/bin/env python3
"""Select only completed, exact-bound rows for a diagnostic comparison."""
from __future__ import annotations

import argparse
import json
from pathlib import Path

from validate_floodgate_diagnostic_summary import validate


COMPARABLE_CLASSES = {"root_score_gap_observed", "tt_move_change_observed", "no_difference_observed"}


def select(document: dict) -> dict:
    errors = validate(document)
    if errors:
        raise ValueError("invalid diagnostic summary: " + ", ".join(errors))
    selected = []
    excluded = []
    for row in document["rows"]:
        reasons = []
        if row["diagnostic_class"] not in COMPARABLE_CLASSES:
            reasons.append("diagnostic_class_not_comparable")
        if row["unrestricted_aborted"] or row["actual_root_aborted"]:
            reasons.append("aborted")
        if row["unrestricted_bound"] != "exact" or row["actual_root_bound"] != "exact":
            reasons.append("bound_not_exact")
        if reasons:
            excluded.append({"index": row.get("index"), "reasons": reasons})
        else:
            selected.append(row)
    return {
        "schema": "sekirei.floodgate-completed-diagnostic-selection.v1",
        "diagnostic_only": True,
        "source_schema": document["schema"],
        "status": "ready" if selected else "not_evaluable",
        "rows": selected,
        "excluded": excluded,
        "counts": {"selected": len(selected), "excluded": len(excluded)},
        "claims": {"strength": "not_permitted", "played_move_is_label": False},
    }


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        result = select(json.loads(args.input.read_text(encoding="utf-8")))
        args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    except (OSError, json.JSONDecodeError, ValueError) as exc:
        parser.error(str(exc))
    print(f"wrote {args.output}: {result['status']} ({result['counts']['selected']} selected)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
