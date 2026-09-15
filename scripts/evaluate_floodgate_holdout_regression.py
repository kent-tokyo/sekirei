#!/usr/bin/env python3
"""Classify a hold-out diagnostic run as clean or inconclusive, never as stronger."""
from __future__ import annotations

import argparse
import json
from pathlib import Path


SCHEMA = "sekirei.floodgate-holdout-regression.v1"
SOURCE_SCHEMA = "sekirei.floodgate-holdout-comparison.v1"


def evaluate(document: dict) -> dict:
    reasons = []
    if document.get("schema") != SOURCE_SCHEMA:
        reasons.append("comparison_schema")
    if document.get("diagnostic_only") is not True:
        reasons.append("diagnostic_only")
    counts = document.get("counts", {})
    total = counts.get("total", 0)
    comparable = counts.get("comparable", 0)
    if not isinstance(total, int) or total < 1 or comparable != total:
        reasons.append("incomplete_rows")
    if not isinstance(document.get("source_corpus_sha256"), str) or not document["source_corpus_sha256"]:
        reasons.append("corpus_provenance_missing_or_mismatched")
    rows = document.get("rows", [])
    if not isinstance(rows, list):
        reasons.append("rows")
        rows = []
    changed_bestmoves = sum(row.get("bestmove_changed", False) for row in rows if isinstance(row, dict))
    nonzero_scores = sum(row.get("score_delta_candidate_minus_baseline_cp", 0) != 0 for row in rows if isinstance(row, dict))
    integrity_statuses = []
    integrity_invalid = False
    for row in rows:
        if not isinstance(row, dict):
            continue
        integrity_present = [
            side in row for side in ("baseline_search_integrity", "candidate_search_integrity")
        ]
        if any(integrity_present) and not all(integrity_present):
            integrity_invalid = True
        for side in ("baseline_search_integrity", "candidate_search_integrity"):
            integrity = row.get(side)
            if isinstance(integrity, dict) and integrity.get("status") in {"verified", "failed", "unknown"}:
                integrity_statuses.append(integrity["status"])
            elif side in row:
                integrity_invalid = True
    if "failed" in integrity_statuses:
        reasons.append("search_integrity_failed")
    elif "unknown" in integrity_statuses:
        reasons.append("search_integrity_unknown")
    if integrity_invalid:
        reasons.append("search_integrity_invalid")
    status = "clean_for_this_diagnostic" if not reasons else "inconclusive"
    return {
        "schema": SCHEMA,
        "diagnostic_only": True,
        "status": status,
        "reasons": reasons,
        "rows": total if isinstance(total, int) else 0,
        "comparable_rows": comparable if isinstance(comparable, int) else 0,
        "bestmove_changes": changed_bestmoves,
        "nonzero_score_deltas": nonzero_scores,
        "search_integrity": {
            "verified": integrity_statuses.count("verified"),
            "failed": integrity_statuses.count("failed"),
            "unknown": integrity_statuses.count("unknown"),
        },
        "claims": {
            "strength": "not_permitted",
            "improvement": "not_established",
            "regression_scope": "fixed-depth diagnostic holdout only",
        },
    }


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("comparison", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    result = evaluate(json.loads(args.comparison.read_text(encoding="utf-8")))
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {result['status']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
