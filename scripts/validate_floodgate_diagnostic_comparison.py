#!/usr/bin/env python3
"""Validate a diagnostic-only comparison produced by the Floodgate tools."""
from __future__ import annotations

import argparse
import json
from pathlib import Path


SCHEMA = "sekirei.floodgate-diagnostic-candidate-comparison.v1"
CLASSES = {
    "root_score_gap_observed", "tt_move_change_observed", "incomplete_search",
    "no_difference_observed",
}


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
    for key in ("baseline_corpus_sha256", "candidate_corpus_sha256"):
        if not isinstance(document.get(key), str) or not document[key]:
            errors.append(key)
    source_corpus = document.get("source_corpus_sha256")
    if source_corpus is not None and (
        not isinstance(source_corpus, str)
        or source_corpus != document.get("baseline_corpus_sha256")
        or source_corpus != document.get("candidate_corpus_sha256")
    ):
        errors.append("source_corpus_sha256")
    rows = document.get("rows")
    if not isinstance(rows, list):
        return errors + ["rows"]
    counts = document.get("counts")
    if not isinstance(counts, dict):
        errors.append("counts")
    else:
        expected = {"total": len(rows), "comparable": 0, "not_comparable": 0}
        expected["comparable"] = sum(row.get("status") == "comparable" for row in rows if isinstance(row, dict))
        expected["not_comparable"] = len(rows) - expected["comparable"]
        for key, value in expected.items():
            if counts.get(key) != value:
                errors.append(f"counts.{key}")
    seen = set()
    for index, row in enumerate(rows):
        prefix = f"rows[{index}]"
        if not isinstance(row, dict):
            errors.append(prefix)
            continue
        key = (row.get("game_id"), row.get("ply"))
        if not isinstance(key[0], str) or not key[0] or not isinstance(key[1], int) or isinstance(key[1], bool):
            errors.append(f"{prefix}.key")
        elif key in seen:
            errors.append(f"{prefix}.duplicate_key")
        seen.add(key)
        if row.get("status") not in {"comparable", "not_comparable"}:
            errors.append(f"{prefix}.status")
        if row.get("baseline_class") not in CLASSES or row.get("candidate_class") not in CLASSES:
            errors.append(f"{prefix}.class")
        reasons = row.get("not_comparable_reasons")
        if not isinstance(reasons, list) or any(not isinstance(reason, str) or not reason for reason in reasons):
            errors.append(f"{prefix}.not_comparable_reasons")
        if not isinstance(row.get("bestmove_changed"), bool):
            errors.append(f"{prefix}.bestmove_changed")
        if not isinstance(row.get("root_candidate_set_changed"), bool):
            errors.append(f"{prefix}.root_candidate_set_changed")
        if not isinstance(row.get("root_candidate_scores_changed"), bool):
            errors.append(f"{prefix}.root_candidate_scores_changed")
        for key_name in ("baseline_root_candidate_moves", "candidate_root_candidate_moves"):
            moves = row.get(key_name)
            if not isinstance(moves, list) or any(not isinstance(move, str) or not move for move in moves):
                errors.append(f"{prefix}.{key_name}")
        deltas = row.get("root_candidate_score_deltas_cp")
        if not isinstance(deltas, dict) or any(not isinstance(move, str) or not isinstance(score, int) or isinstance(score, bool)
                                                for move, score in deltas.items()):
            errors.append(f"{prefix}.root_candidate_score_deltas_cp")
    return errors


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("comparison", type=Path)
    args = parser.parse_args(argv)
    document = json.loads(args.comparison.read_text(encoding="utf-8"))
    errors = validate(document)
    if errors:
        print("invalid diagnostic comparison: " + ", ".join(errors))
        return 1
    print(f"valid diagnostic comparison: {args.comparison}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
