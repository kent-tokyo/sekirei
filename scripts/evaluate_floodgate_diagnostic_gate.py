#!/usr/bin/env python3
"""Evaluate whether diagnostic evidence is sufficient to enter a hold-out gate."""
from __future__ import annotations

import argparse
import json
from pathlib import Path


SCHEMA = "sekirei.floodgate-diagnostic-gate-decision.v1"
COMPARISON_SCHEMAS = {
    "sekirei.floodgate-diagnostic-candidate-comparison.v1",
    "sekirei.floodgate-holdout-comparison.v1",
}


def evaluate(comparison: dict, split: dict) -> dict:
    errors = []
    comparison_schema = comparison.get("schema")
    if comparison_schema not in COMPARISON_SCHEMAS:
        errors.append("comparison_schema")
    if comparison.get("diagnostic_only") is not True or comparison.get("decision") != "not_evaluable":
        errors.append("comparison_boundary")
    if split.get("schema") != "sekirei.floodgate-diagnostic-corpus-split.v1":
        errors.append("split_schema")
    comparison_hash = comparison.get("source_corpus_sha256")
    split_hash = split.get("source_corpus_sha256")
    if isinstance(comparison_hash, str) and isinstance(split_hash, str) and comparison_hash != split_hash:
        errors.append("corpus_hash_mismatch")
    counts = comparison.get("counts", {})
    comparable = counts.get("comparable", 0)
    holdout = split.get("holdout_entry_indices", [])
    if not isinstance(comparable, int) or comparable < 1:
        errors.append("no_comparable_rows")
    if not isinstance(holdout, list) or len(holdout) < 2:
        errors.append("insufficient_holdout_rows")
    status = "ready_for_holdout" if not errors else "not_ready"
    return {
        "schema": SCHEMA,
        "diagnostic_only": True,
        "status": status,
        "reasons": errors,
        "comparison_rows": comparable if isinstance(comparable, int) else 0,
        "holdout_rows": len(holdout) if isinstance(holdout, list) else 0,
        "source_corpus_sha256": split_hash if isinstance(split_hash, str) else None,
        "next_step": (
            "review formal gate contract"
            if status == "ready_for_holdout" and comparison_schema == "sekirei.floodgate-holdout-comparison.v1"
            else "run independent hold-out regression"
            if status == "ready_for_holdout"
            else "collect complete comparable diagnostics and independent hold-out"
        ),
        "claims": {"strength": "not_permitted", "release_approval": "not_granted"},
    }


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("comparison", type=Path)
    parser.add_argument("split", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    result = evaluate(json.loads(args.comparison.read_text(encoding="utf-8")),
                      json.loads(args.split.read_text(encoding="utf-8")))
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {result['status']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
