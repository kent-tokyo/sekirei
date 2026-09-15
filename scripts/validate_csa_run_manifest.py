#!/usr/bin/env python3
"""Validate a CSA startup/finalized run manifest without exposing credentials."""
from __future__ import annotations

import json
import sys
from pathlib import Path


SCHEMA = "sekirei.csa-run-manifest.v1"
DEFERRED = "deferred_to_finalize_csa_run_manifest"
REQUIRED = ("engine", "engine_version", "source_revision", "evaluation", "search_backend",
            "hash_mb", "max_depth", "resign_score_cp", "ponder", "game_id", "server", "port")


def validate(document: dict, finalized: bool = False) -> list[str]:
    errors = []
    if document.get("schema") != SCHEMA:
        errors.append("schema")
    if document.get("status") not in {"active", "finalized"}:
        errors.append("status")
    for key in REQUIRED:
        if key not in document:
            errors.append(key)
    if document.get("evaluation") not in {"material", "nnue"}:
        errors.append("evaluation")
    if document.get("search_backend") != "alpha_beta":
        errors.append("search_backend")
    for key in ("hash_mb", "max_depth", "resign_score_cp", "port"):
        if not isinstance(document.get(key), int):
            errors.append(key)
    if not isinstance(document.get("ponder"), str):
        errors.append("ponder")
    if "analysis_record_schema" in document and document["analysis_record_schema"] not in {"sekirei.analysis-record.v2", "sekirei.analysis-record.v3"}:
        errors.append("analysis_record_schema")
    if "keep_alive" in document and not isinstance(document["keep_alive"], bool):
        errors.append("keep_alive")
    if "games" in document:
        games = document["games"]
        if not isinstance(games, list) or not games:
            errors.append("games")
        elif any(not isinstance(game, dict) or not isinstance(game.get("game_id"), str)
                 or not game["game_id"] for game in games):
            errors.append("games.game_id")
    for key in ("record_dir", "analysis_dir"):
        if key in document and document[key] is not None and not isinstance(document[key], (str, list)):
            errors.append(key)
    binary = document.get("binary")
    if not isinstance(binary, dict) or not isinstance(binary.get("path"), (str, list)):
        errors.append("binary.path")
    elif not isinstance(binary.get("bytes"), int) or binary["bytes"] <= 0:
        errors.append("binary.bytes")
    weights = document.get("weights")
    if not isinstance(weights, dict):
        errors.append("weights")
    else:
        active = weights.get("active")
        if not isinstance(active, bool):
            errors.append("weights.active")
        if document.get("evaluation") == "nnue":
            if not isinstance(weights.get("path"), (str, list)) or not active:
                errors.append("weights.nnue_active")
        elif active:
            errors.append("weights.material_active")
    if finalized:
        if document.get("status") != "finalized":
            errors.append("status.finalized")
        if isinstance(binary, dict) and (not isinstance(binary.get("sha256"), str)
                                         or len(binary["sha256"]) != 64
                                         or binary["sha256"] == DEFERRED):
            errors.append("binary.sha256")
        if document.get("evaluation") == "nnue":
            if not isinstance(weights, dict) or not isinstance(weights.get("sha256"), str) \
                    or len(weights["sha256"]) != 64 or weights["sha256"] == DEFERRED:
                errors.append("weights.sha256")
        if document.get("source_revision") in (None, "unknown"):
            errors.append("source_revision.finalized")
    return errors


def main(argv=None) -> int:
    args = list(argv or sys.argv[1:])
    finalized = "--finalized" in args
    args = [arg for arg in args if arg != "--finalized"]
    if len(args) != 1:
        print(f"usage: {Path(sys.argv[0]).name} [--finalized] MANIFEST.json", file=sys.stderr)
        return 2
    document = json.loads(Path(args[0]).read_text(encoding="utf-8"))
    errors = validate(document, finalized=finalized)
    if errors:
        print("invalid CSA run manifest: " + ", ".join(errors), file=sys.stderr)
        return 1
    print(f"valid CSA run manifest: {args[0]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
