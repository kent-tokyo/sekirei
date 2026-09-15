#!/usr/bin/env python3
"""Validate a diagnostic-only Floodgate summary artifact."""
from __future__ import annotations

import json
import sys
from pathlib import Path


SCHEMA = "sekirei.floodgate-diagnostic-summary.v1"
CLASSES = {
    "root_score_gap_observed",
    "tt_move_change_observed",
    "incomplete_search",
    "no_difference_observed",
}


def validate(document: dict) -> list[str]:
    errors = []
    if document.get("schema") != SCHEMA:
        errors.append("schema")
    if document.get("diagnostic_only") is not True:
        errors.append("diagnostic_only")
    claims = document.get("claims")
    if not isinstance(claims, dict) or claims.get("strength") != "not_permitted":
        errors.append("claims.strength")
    execution = document.get("execution")
    if not isinstance(execution, dict):
        errors.append("execution")
    else:
        for key in ("source_revision", "binary", "options", "corpus_sha256"):
            if key not in execution:
                errors.append(f"execution.{key}")
        if "weights" not in execution:
            errors.append("execution.weights")
        if not isinstance(execution.get("binary"), dict) or not execution["binary"].get("sha256"):
            errors.append("execution.binary.sha256")
        if execution.get("weights") is not None and not isinstance(execution.get("weights"), dict):
            errors.append("execution.weights")
        if not isinstance(execution.get("options"), dict):
            errors.append("execution.options")
        if not isinstance(execution.get("corpus_sha256"), str) or not execution["corpus_sha256"]:
            errors.append("execution.corpus_sha256")
    for key in ("nodes", "warmup_nodes"):
        if not isinstance(document.get(key), int) or document[key] < 0:
            errors.append(key)
    rows = document.get("rows")
    if not isinstance(rows, list):
        return errors + ["rows"]
    for index, row in enumerate(rows):
        prefix = f"rows[{index}]"
        if not isinstance(row, dict):
            errors.append(prefix)
            continue
        if row.get("diagnostic_class") not in CLASSES:
            errors.append(f"{prefix}.diagnostic_class")
        for key in ("unrestricted_aborted", "actual_root_aborted"):
            if not isinstance(row.get(key), bool):
                errors.append(f"{prefix}.{key}")
        for key in ("unrestricted_pv", "actual_root_pv"):
            if not isinstance(row.get(key), list):
                errors.append(f"{prefix}.{key}")
        if "root_candidates" in row:
            candidates = row["root_candidates"]
            if not isinstance(candidates, list):
                errors.append(f"{prefix}.root_candidates")
            else:
                seen = set()
                for candidate_index, candidate in enumerate(candidates):
                    candidate_prefix = f"{prefix}.root_candidates[{candidate_index}]"
                    if not isinstance(candidate, dict):
                        errors.append(candidate_prefix)
                        continue
                    move = candidate.get("move")
                    if not isinstance(move, str) or not move or move in seen:
                        errors.append(f"{candidate_prefix}.move")
                    seen.add(move)
                    if not isinstance(candidate.get("score_cp"), int) or isinstance(candidate.get("score_cp"), bool):
                        errors.append(f"{candidate_prefix}.score_cp")
                    if not isinstance(candidate.get("depth"), int) or isinstance(candidate.get("depth"), bool) or candidate["depth"] < 0:
                        errors.append(f"{candidate_prefix}.depth")
                    if candidate.get("bound") not in {"exact", "lower", "upper", "unknown"}:
                        errors.append(f"{candidate_prefix}.bound")
                    if not isinstance(candidate.get("abort_reason"), str) or not candidate["abort_reason"]:
                        errors.append(f"{candidate_prefix}.abort_reason")
    return errors


def main(argv=None) -> int:
    args = argv or sys.argv[1:]
    if len(args) != 1:
        print(f"usage: {Path(sys.argv[0]).name} SUMMARY.json", file=sys.stderr)
        return 2
    document = json.loads(Path(args[0]).read_text(encoding="utf-8"))
    errors = validate(document)
    if errors:
        print("invalid diagnostic summary: " + ", ".join(errors), file=sys.stderr)
        return 1
    print(f"valid diagnostic summary: {args[0]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
