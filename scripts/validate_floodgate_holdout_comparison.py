#!/usr/bin/env python3
"""Validate a diagnostic-only hold-out comparison artifact."""
from __future__ import annotations

import argparse
import json
import re
from pathlib import Path


SCHEMA = "sekirei.floodgate-holdout-comparison.v1"
INTEGRITY_STATUSES = {"verified", "failed", "unknown"}
SHA256 = re.compile(r"^[0-9a-f]{64}$")


def _validate_integrity(value: object, prefix: str) -> list[str]:
    if not isinstance(value, dict) or value.get("status") not in INTEGRITY_STATUSES:
        return [f"{prefix}.status"]
    status = value["status"]
    if status == "unknown":
        if not isinstance(value.get("missing"), list) or not value["missing"]:
            return [f"{prefix}.missing"]
        return []
    fields = ("pv_legal", "pv_replay_preserves_input")
    if any(not isinstance(value.get(field), bool) for field in fields):
        return [f"{prefix}.{field}" for field in fields if not isinstance(value.get(field), bool)]
    expected = status == "verified"
    if all(value[field] is expected for field in fields):
        return []
    return [f"{prefix}.status_values"]


def validate(document: dict) -> list[str]:
    errors = []
    if document.get("schema") != SCHEMA:
        errors.append("schema")
    if document.get("diagnostic_only") is not True:
        errors.append("diagnostic_only")
    if document.get("decision") != "not_evaluable":
        errors.append("decision")
    claims = document.get("claims")
    if not isinstance(claims, dict) or claims.get("strength") != "not_permitted":
        errors.append("claims.strength")
    if claims.get("played_move_is_label") is not False:
        errors.append("claims.played_move_is_label")
    for field in ("baseline_corpus_sha256", "candidate_corpus_sha256"):
        if not isinstance(document.get(field), str) or not SHA256.fullmatch(document[field]):
            errors.append(field)
    source_hash = document.get("source_corpus_sha256")
    baseline_hash = document.get("baseline_corpus_sha256")
    candidate_hash = document.get("candidate_corpus_sha256")
    if baseline_hash != candidate_hash and source_hash is not None:
        errors.append("source_corpus_sha256")
    elif baseline_hash == candidate_hash and source_hash != baseline_hash:
        errors.append("source_corpus_sha256")
    for field in ("execution_differences", "allowed_execution_differences"):
        value = document.get(field)
        if not isinstance(value, list) or any(not isinstance(item, str) or not item for item in value):
            errors.append(field)

    rows = document.get("rows")
    if not isinstance(rows, list) or not rows:
        return errors + ["rows"]
    seen = set()
    for index, row in enumerate(rows):
        prefix = f"rows[{index}]"
        if not isinstance(row, dict):
            errors.append(prefix)
            continue
        key = row.get("key")
        if not isinstance(key, str) or not key:
            errors.append(f"{prefix}.key")
        elif key in seen:
            errors.append(f"{prefix}.duplicate_key")
        seen.add(key)
        if row.get("status") not in {"comparable", "not_comparable"}:
            errors.append(f"{prefix}.status")
        reasons = row.get("not_comparable_reasons")
        if not isinstance(reasons, list) or any(not isinstance(reason, str) or not reason for reason in reasons):
            errors.append(f"{prefix}.not_comparable_reasons")
        for field in ("bestmove_changed", "pv_changed"):
            if not isinstance(row.get(field), bool):
                errors.append(f"{prefix}.{field}")
        for field in ("score_delta_candidate_minus_baseline_cp", "depth_delta_candidate_minus_baseline"):
            if not isinstance(row.get(field), int) or isinstance(row[field], bool):
                errors.append(f"{prefix}.{field}")
        integrity_fields = ("baseline_search_integrity", "candidate_search_integrity")
        present = [field in row for field in integrity_fields]
        if any(present):
            if not all(present):
                errors.append(f"{prefix}.search_integrity_pair")
            else:
                for field in integrity_fields:
                    errors.extend(_validate_integrity(row[field], f"{prefix}.{field}"))

    counts = document.get("counts")
    expected = {
        "total": len(rows),
        "comparable": sum(row.get("status") == "comparable" for row in rows if isinstance(row, dict)),
        "not_comparable": sum(row.get("status") == "not_comparable" for row in rows if isinstance(row, dict)),
    }
    if not isinstance(counts, dict):
        errors.append("counts")
    elif counts != expected:
        errors.append("counts")
    return errors


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("comparison", type=Path)
    args = parser.parse_args(argv)
    try:
        document = json.loads(args.comparison.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        print(f"invalid hold-out comparison: {exc}")
        return 2
    errors = validate(document)
    if errors:
        print("invalid hold-out comparison: " + ", ".join(errors))
        return 1
    print(f"valid hold-out comparison: {args.comparison}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
