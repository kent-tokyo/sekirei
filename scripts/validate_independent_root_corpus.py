#!/usr/bin/env python3
"""Validate a frozen independent fixed-depth root-comparison corpus.

The validator does not judge a teacher's moves.  It binds the source subset,
history-aware corpus, fixed-depth teacher output, and strict pairs so a later
candidate experiment cannot silently reuse the earlier development holdout.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source_paths(document: dict) -> set[str]:
    if isinstance(document.get("sources"), list):
        return {row["path"] for row in document["sources"] if isinstance(row, dict) and isinstance(row.get("path"), str)}
    if isinstance(document.get("entries"), list):
        return {row["source"] for row in document["entries"] if isinstance(row, dict) and isinstance(row.get("source"), str)}
    return set()


def validate(subset: dict, prior: dict, corpus: dict, teacher: dict, pairs: dict,
             corpus_path: Path) -> list[str]:
    errors: list[str] = []
    subset_paths, prior_paths = source_paths(subset), source_paths(prior)
    if subset.get("schema") != "sekirei.hashed-csa-subset.v1" or not subset_paths:
        errors.append("subset")
    if not prior_paths:
        errors.append("prior_sources")
    if subset_paths & prior_paths:
        errors.append("prior_source_overlap")
    if corpus.get("schema") != "sekirei.history-aware-csa-diagnostic.v1":
        errors.append("corpus.schema")
    if corpus.get("diagnostic_only") is not True or corpus.get("invalid_replays"):
        errors.append("corpus.validity")
    positions = corpus.get("positions")
    if not isinstance(positions, list) or not positions:
        errors.append("corpus.positions")
    else:
        identities = {(row.get("initial_sfen"), tuple(row.get("history_before_usi", []))) for row in positions}
        if len(identities) != len(positions):
            errors.append("corpus.duplicate_history")
    if teacher.get("schema") != "sekirei.root-rank-teacher-corpus.v1":
        errors.append("teacher.schema")
    if teacher.get("diagnostic_only") is not True:
        errors.append("teacher.claim")
    if teacher.get("source_corpus", {}).get("sha256") != sha256(corpus_path):
        errors.append("teacher.corpus_hash")
    rows = teacher.get("rows")
    if not isinstance(rows, list) or not rows or not all(row.get("candidate_prefix_complete") is True for row in rows):
        errors.append("teacher.incomplete_prefix")
    if pairs.get("schema") != "sekirei.root-rank-pairs.v1" or pairs.get("diagnostic_only") is not True:
        errors.append("pairs.schema")
    pair_rows = pairs.get("pairs")
    parent_ids = {row.get("id") for row in rows} if isinstance(rows, list) else set()
    if not isinstance(pair_rows, list) or not pair_rows:
        errors.append("pairs.empty")
    elif any(
        pair.get("parent_id") not in parent_ids
        or not isinstance(pair.get("teacher_score_gap_cp"), int)
        or pair["teacher_score_gap_cp"] <= 0
        for pair in pair_rows
    ):
        errors.append("pairs.invalid")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--subset", type=Path, required=True)
    parser.add_argument("--prior", type=Path, action="append", required=True,
                        help="previous subset/holdout manifest; repeat to exclude multiple ranges")
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--teacher", type=Path, required=True)
    parser.add_argument("--pairs", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    subset = json.loads(args.subset.read_text(encoding="utf-8"))
    prior_documents = [json.loads(path.read_text(encoding="utf-8")) for path in args.prior]
    prior = {"sources": [
        {"path": source}
        for document in prior_documents for source in sorted(source_paths(document))
    ]}
    corpus = json.loads(args.corpus.read_text(encoding="utf-8"))
    teacher = json.loads(args.teacher.read_text(encoding="utf-8"))
    pairs = json.loads(args.pairs.read_text(encoding="utf-8"))
    errors = validate(subset, prior, corpus, teacher, pairs, args.corpus)
    output = {
        "schema": "sekirei.independent-root-corpus-audit.v1",
        "status": "valid" if not errors else "invalid",
        "diagnostic_only": True,
        "strength_claim": "not_permitted",
        "inputs": {
            name: {"path": str(path), "sha256": sha256(path)}
            for name, path in (("subset", args.subset), ("corpus", args.corpus),
                               ("teacher", args.teacher), ("pairs", args.pairs))
        },
        "prior_inputs": [{"path": str(path), "sha256": sha256(path)} for path in args.prior],
        "errors": errors,
    }
    args.output.write_text(json.dumps(output, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {output['status']}")
    return 0 if not errors else 1


if __name__ == "__main__":
    raise SystemExit(main())
