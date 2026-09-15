#!/usr/bin/env python3
"""Validate a replay-derived Floodgate diagnostic corpus."""
from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path


HEX64 = re.compile(r"^[0-9a-f]{64}$")
ENTRY_KINDS = {"swing", "control"}


def _hash_matches(path: Path, expected: object) -> bool:
    return isinstance(expected, str) and HEX64.fullmatch(expected) is not None \
        and hashlib.sha256(path.read_bytes()).hexdigest() == expected


def _text_hash_matches(value: object, expected: object) -> bool:
    return isinstance(value, str) and isinstance(expected, str) \
        and HEX64.fullmatch(expected) is not None \
        and hashlib.sha256(value.encode("utf-8")).hexdigest() == expected


def validate(document: object, root: Path | None = None, verify_sources: bool = False) -> list[str]:
    if not isinstance(document, dict):
        return ["document must be an object"]
    errors: list[str] = []
    if document.get("schema") != "sekirei.floodgate-diagnostic-corpus.v1":
        errors.append("schema")
    if document.get("diagnostic_only") is not True:
        errors.append("diagnostic_only")
    if document.get("split") != "diagnostic_tuning_only":
        errors.append("split")
    if document.get("strength_claim") != "not_permitted":
        errors.append("strength_claim")
    entries = document.get("entries")
    if not isinstance(entries, list) or not entries:
        return errors + ["entries"]
    seen: set[tuple[object, object]] = set()
    for index, entry in enumerate(entries):
        prefix = f"entries[{index}]"
        if not isinstance(entry, dict):
            errors.append(f"{prefix}.object")
            continue
        if entry.get("entry_kind") not in ENTRY_KINDS:
            errors.append(f"{prefix}.entry_kind")
        if entry.get("label_policy") != "observation_only_no_correct_move_label":
            errors.append(f"{prefix}.label_policy")
        source = entry.get("source")
        position = entry.get("position")
        if not isinstance(source, dict) or not isinstance(position, dict):
            errors.append(f"{prefix}.source_position")
            continue
        for field in ("csa_sha256", "analysis_sha256"):
            if not isinstance(source.get(field), str) or HEX64.fullmatch(source[field]) is None:
                errors.append(f"{prefix}.source.{field}")
        ply = source.get("ply")
        if not isinstance(ply, int) or isinstance(ply, bool) or ply < 0:
            errors.append(f"{prefix}.source.ply")
        key = (source.get("game_id"), ply)
        if key in seen:
            errors.append(f"{prefix}.duplicate_source_ply")
        seen.add(key)
        sfen = position.get("sfen")
        if not isinstance(sfen, str) or len(sfen.split()) != 4:
            errors.append(f"{prefix}.position.sfen")
        elif not _text_hash_matches(sfen, position.get("sfen_sha256")):
            errors.append(f"{prefix}.position.sfen_sha256")
        if position.get("side_to_move") not in {"black", "white"}:
            errors.append(f"{prefix}.position.side_to_move")
        history = position.get("history_before")
        if not isinstance(history, list):
            errors.append(f"{prefix}.position.history_before")
        elif isinstance(ply, int) and len(history) != ply:
            errors.append(f"{prefix}.position.history_length")
        moves_before = position.get("moves_before")
        if not isinstance(moves_before, list):
            errors.append(f"{prefix}.position.moves_before")
        elif isinstance(history, list) and moves_before != history[max(0, len(history) - 2):]:
            errors.append(f"{prefix}.position.moves_history_suffix")
        if verify_sources:
            if root is None:
                errors.append("root required for source verification")
            else:
                for path_field, hash_field in (("csa", "csa_sha256"), ("analysis", "analysis_sha256")):
                    path = root / str(source.get(path_field, ""))
                    try:
                        matches = _hash_matches(path, source.get(hash_field))
                    except OSError:
                        matches = False
                    if not matches:
                        errors.append(f"{prefix}.source.{path_field}_hash")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("corpus", type=Path)
    parser.add_argument("--verify-sources", action="store_true")
    parser.add_argument("--root", type=Path, default=Path.cwd())
    args = parser.parse_args()
    try:
        document = json.loads(args.corpus.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        print(f"invalid diagnostic corpus: {error}")
        return 1
    errors = validate(document, root=args.root, verify_sources=args.verify_sources)
    if errors:
        print("invalid diagnostic corpus: " + "; ".join(errors))
        return 1
    print(f"diagnostic corpus valid: {args.corpus}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
