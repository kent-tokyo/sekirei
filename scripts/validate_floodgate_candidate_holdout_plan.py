#!/usr/bin/env python3
"""Validate a non-executing current-candidate hold-out plan."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


SCHEMA = "sekirei.floodgate-candidate-holdout-plan.v1"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def validate(document: dict) -> list[str]:
    errors = []
    if document.get("schema") != SCHEMA:
        errors.append("schema")
    if document.get("status") != "planned":
        errors.append("status")
    if document.get("execution_performed") is not False:
        errors.append("execution_performed")
    candidate = document.get("candidate")
    if not isinstance(candidate, dict) or not isinstance(candidate.get("version"), str) or not candidate["version"]:
        errors.append("candidate.version")
    split = document.get("split")
    if not isinstance(split, dict) or not isinstance(split.get("holdout_entries"), int) or split["holdout_entries"] < 2:
        errors.append("split.holdout_entries")
    protocol = document.get("protocol")
    if not isinstance(protocol, dict):
        errors.append("protocol")
    else:
        expected = {"Threads": "1", "SpecTopN": "0", "UseBook": "false", "SearchMode": "Speculative"}
        for key, value in expected.items():
            if protocol.get(key) != value:
                errors.append(f"protocol.{key}")
        if protocol.get("preflight_required") is not True:
            errors.append("protocol.preflight_required")
        if protocol.get("formal_gate") is not False:
            errors.append("protocol.formal_gate")
    claims = document.get("claims")
    if not isinstance(claims, dict) or claims.get("strength") != "not_permitted" or claims.get("candidate_adoption") != "not_established":
        errors.append("claims")
    return errors


def verify_artifacts(document: dict, root: Path) -> list[str]:
    errors = []
    candidate = document.get("candidate", {})
    corpus = document.get("corpus", {})
    split = document.get("split", {})
    candidate_path = root / candidate.get("manifest", "") if isinstance(candidate, dict) else root
    corpus_path = root / corpus.get("path", "") if isinstance(corpus, dict) else root
    split_path = root / split.get("path", "") if isinstance(split, dict) else root
    for name, path in (("candidate.manifest", candidate_path), ("corpus.path", corpus_path), ("split.path", split_path)):
        if not path.is_file():
            errors.append(name)
    if errors:
        return errors
    candidate_document = json.loads(candidate_path.read_text(encoding="utf-8"))
    if candidate_document.get("run_contract", {}).get("engine_version") != candidate.get("version"):
        errors.append("candidate.version")
    expected_file_hash = corpus.get("file_sha256")
    if not isinstance(expected_file_hash, str) or sha256(corpus_path) != expected_file_hash:
        errors.append("corpus.file_sha256")
    split_document = json.loads(split_path.read_text(encoding="utf-8"))
    if split_document.get("source_corpus_sha256") != corpus.get("source_corpus_sha256"):
        errors.append("corpus.source_corpus_sha256")
    if split_document.get("holdout_entry_indices") and len(split_document["holdout_entry_indices"]) != split.get("holdout_entries"):
        errors.append("split.holdout_entries")
    return errors


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("plan", type=Path)
    parser.add_argument("--verify-artifacts", action="store_true")
    parser.add_argument("--root", type=Path, default=Path.cwd())
    args = parser.parse_args(argv)
    document = json.loads(args.plan.read_text(encoding="utf-8"))
    errors = validate(document)
    if args.verify_artifacts:
        errors.extend(verify_artifacts(document, args.root))
    if errors:
        print("invalid candidate hold-out plan: " + ", ".join(errors))
        return 1
    print(f"candidate hold-out plan valid: {args.plan}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
