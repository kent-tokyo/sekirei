#!/usr/bin/env python3
"""Validate actual book-decision logs against a preregistered coverage gate."""
from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter
from pathlib import Path
from typing import Any


DECLARATION_SCHEMA = "sekirei.book-coverage-declaration.v1"
REPORT_SCHEMA = "sekirei.book-coverage-preflight.v1"


class ContractError(ValueError):
    pass


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ContractError(f"JSON object required: {path}")
    return value


def resolve_artifact(root: Path, declaration: dict[str, Any], name: str) -> Path:
    entry = declaration.get("artifacts", {}).get(name)
    if not isinstance(entry, dict) or not isinstance(entry.get("path"), str):
        raise ContractError(f"missing artifact declaration: {name}")
    path = (root / entry["path"]).resolve(strict=True)
    root = root.resolve()
    if path != root and root not in path.parents:
        raise ContractError(f"artifact escapes declaration root: {name}")
    if not path.is_file() or sha256(path) != entry.get("sha256"):
        raise ContractError(f"artifact hash mismatch: {name}")
    return path


def declaration_digest(declaration: dict[str, Any]) -> str:
    unsigned = {key: value for key, value in declaration.items() if key != "declaration_sha256"}
    payload = json.dumps(unsigned, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(payload).hexdigest()


def validate_declaration(path: Path) -> tuple[dict[str, Any], dict[str, Path]]:
    declaration = load_json(path)
    if declaration.get("schema") != DECLARATION_SCHEMA:
        raise ContractError("unsupported declaration schema")
    if declaration.get("declaration_sha256") != declaration_digest(declaration):
        raise ContractError("declaration SHA-256 mismatch")
    protocol = declaration.get("protocol")
    if not isinstance(protocol, dict):
        raise ContractError("protocol is required")
    for field in ("positions", "games_per_position", "max_moves", "threads"):
        if not isinstance(protocol.get(field), int) or protocol[field] <= 0:
            raise ContractError(f"protocol.{field} must be positive")
    if protocol["max_moves"] != 1 or protocol["threads"] != 1:
        raise ContractError("coverage preflight must use one ply and one thread")
    thresholds = declaration.get("thresholds")
    if not isinstance(thresholds, dict):
        raise ContractError("thresholds are required")
    minimum = thresholds.get("minimum_book_selections")
    fraction = thresholds.get("minimum_selection_fraction")
    if not isinstance(minimum, int) or minimum <= 0:
        raise ContractError("minimum_book_selections must be positive")
    if isinstance(fraction, bool) or not isinstance(fraction, (int, float)) or not 0 < fraction <= 1:
        raise ContractError("minimum_selection_fraction must be in (0, 1]")
    identities = declaration.get("identities")
    if not isinstance(identities, dict):
        raise ContractError("identities are required")
    engine = identities.get("engine")
    if not isinstance(engine, dict) or not isinstance(engine.get("sha256"), str) or len(engine["sha256"]) != 64:
        raise ContractError("identities.engine.sha256 is required")
    required = ("split_manifest", "heldout_manifest", "openings", "book", "book_manifest")
    artifacts = {name: resolve_artifact(path.parent, declaration, name) for name in required}
    return declaration, artifacts


def summarize(declaration_path: Path, log_paths: list[Path]) -> dict[str, Any]:
    declaration, artifacts = validate_declaration(declaration_path)
    expected_book_hash = sha256(artifacts["book"])
    decisions: list[dict[str, Any]] = []
    fallback = Counter()
    for log_path in log_paths:
        for line_number, line in enumerate(log_path.read_text(encoding="utf-8").splitlines(), 1):
            row = json.loads(line)
            if row.get("event") != "decision":
                continue
            if row.get("schema") != "sekirei-book-decision-v1" or row.get("use_book") is not True:
                raise ContractError(f"malformed decision at {log_path}:{line_number}")
            if row.get("book_sha256") != expected_book_hash:
                raise ContractError(f"book hash mismatch at {log_path}:{line_number}")
            if row.get("ply") != 0:
                raise ContractError(f"preflight decision is not the first searched ply at {log_path}:{line_number}")
            decisions.append(row)
            if row.get("fallback_reason") is not None:
                fallback[str(row["fallback_reason"])] += 1
    expected = declaration["protocol"]["positions"] * declaration["protocol"]["games_per_position"]
    if len(decisions) != expected:
        raise ContractError(f"expected {expected} first-ply decisions, found {len(decisions)}")
    state_hashes = [row.get("state_sha256") for row in decisions]
    if not all(isinstance(value, str) and value for value in state_hashes):
        raise ContractError("decision state hashes are missing")
    selected = sum(row.get("selected_action") is not None for row in decisions)
    coverage = selected / len(decisions)
    thresholds = declaration["thresholds"]
    ready = selected >= thresholds["minimum_book_selections"] and coverage >= thresholds["minimum_selection_fraction"]
    return {
        "schema": REPORT_SCHEMA,
        "declaration_sha256": declaration["declaration_sha256"],
        "source_split_disjoint": True,
        "actual_decisions": len(decisions),
        "unique_states": len(set(state_hashes)),
        "book_selections": selected,
        "abstentions": len(decisions) - selected,
        "selection_fraction": coverage,
        "fallback_reasons": dict(sorted(fallback.items())),
        "thresholds": thresholds,
        "status": "ready" if ready else "not_ready",
        "full_gate_started": False,
        "gate_verdict": "NOT_RUN",
        "interpretation": "INCONCLUSIVE",
        "strength_claim_permitted": False,
        "next_action": "run_preregistered_full_gate" if ready else "improve_book_support_before_full_gate",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--declaration", type=Path, required=True)
    parser.add_argument("--decision-log", type=Path, action="append", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        report = summarize(args.declaration, args.decision_log)
    except (OSError, json.JSONDecodeError, ContractError) as error:
        raise SystemExit(f"invalid book coverage preflight: {error}") from error
    args.output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
