#!/usr/bin/env python3
"""Validate the diagnostic-only Floodgate root-review artifact."""
from __future__ import annotations

import hashlib
import json
import re
import sys
from pathlib import Path


HEX64 = re.compile(r"^[0-9a-f]{64}$")


def _matches(path: Path, expected: str) -> bool:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest() == expected


def validate(document: dict, root: Path | None = None, verify_artifacts: bool = False) -> list[str]:
    errors = []
    schema = document.get("schema")
    if schema not in {"sekirei.floodgate-root-review.v1", "sekirei.floodgate-root-review.v2"}:
        errors.append("schema")
    if document.get("diagnostic_only") is not True:
        errors.append("diagnostic_only")
    if document.get("strength_claim") != "not_permitted":
        errors.append("strength_claim")
    contract = document.get("contract")
    if not isinstance(contract, dict):
        errors.append("contract")
    else:
        for key in ("nodes", "max_depth"):
            if not isinstance(contract.get(key), int) or contract[key] <= 0:
                errors.append(f"contract.{key}")
        if contract.get("threads") != 1:
            errors.append("contract.threads")
        if contract.get("spec_top_n") != 0:
            errors.append("contract.spec_top_n")
        if contract.get("played_move_is_label") is not False:
            errors.append("contract.played_move_is_label")
        if "disable_nmp" in contract and not isinstance(contract["disable_nmp"], bool):
            errors.append("contract.disable_nmp")
    for key in ("binary", "history_binary"):
        artifact = document.get(key)
        if not isinstance(artifact, dict) or not isinstance(artifact.get("path"), str) or not artifact["path"]:
            errors.append(f"{key}.path")
        if not isinstance(artifact, dict) or not HEX64.fullmatch(artifact.get("sha256", "")):
            errors.append(f"{key}.sha256")
        if verify_artifacts and root is not None and isinstance(artifact, dict):
            path = root / artifact.get("path", "")
            try:
                if not _matches(path, artifact.get("sha256", "")):
                    errors.append(f"{key}.hash_mismatch")
            except OSError:
                errors.append(f"{key}.missing")
    rows = document.get("rows")
    if not isinstance(rows, list) or not rows:
        errors.append("rows")
        return errors
    for index, row in enumerate(rows):
        prefix = f"rows[{index}]"
        if not isinstance(row, dict):
            errors.append(prefix)
            continue
        for key in ("game_id", "result", "played_move_usi"):
            if not isinstance(row.get(key), str) or not row[key]:
                errors.append(f"{prefix}.{key}")
        if not isinstance(row.get("ply"), int) or row["ply"] < 0:
            errors.append(f"{prefix}.ply")
        if row.get("played_move_is_label") is not False:
            errors.append(f"{prefix}.played_move_is_label")
        source = row.get("source")
        if not isinstance(source, dict):
            errors.append(f"{prefix}.source")
        else:
            for key in ("csa", "analysis"):
                if not isinstance(source.get(key), str) or not source[key]:
                    errors.append(f"{prefix}.source.{key}")
            for key in ("csa_sha256", "analysis_sha256"):
                if not HEX64.fullmatch(source.get(key, "")):
                    errors.append(f"{prefix}.source.{key}")
            if verify_artifacts and root is not None:
                for path_key, hash_key in (("csa", "csa_sha256"), ("analysis", "analysis_sha256")):
                    path = root / source.get(path_key, "")
                    try:
                        if not _matches(path, source.get(hash_key, "")):
                            errors.append(f"{prefix}.source.{path_key}_hash_mismatch")
                    except OSError:
                        errors.append(f"{prefix}.source.{path_key}_missing")
        replay = row.get("history_replay")
        if not isinstance(replay, dict) or replay.get("status") != "verified":
            errors.append(f"{prefix}.history_replay")
        for key in ("cold", "warm", "actual_root"):
            result = row.get(key)
            if not isinstance(result, dict) or result.get("completion") != "search_completed":
                errors.append(f"{prefix}.{key}")
            elif schema == "sekirei.floodgate-root-review.v2":
                if not isinstance(result.get("bestmove"), str) or not result["bestmove"]:
                    errors.append(f"{prefix}.{key}.bestmove")
                if not isinstance(result.get("depth"), int) or result["depth"] <= 0:
                    errors.append(f"{prefix}.{key}.depth")
                if not isinstance(result.get("nodes"), int) or result["nodes"] <= 0:
                    errors.append(f"{prefix}.{key}.nodes")
                if not isinstance(result.get("score_cp"), int):
                    errors.append(f"{prefix}.{key}.score_cp")
                if key == "actual_root" and result.get("root_move_usi") != row.get("played_move_usi"):
                    errors.append(f"{prefix}.actual_root.root_move_usi")
        comparison = row.get("comparison")
        if not isinstance(comparison, dict):
            errors.append(f"{prefix}.comparison")
        else:
            if schema == "sekirei.floodgate-root-review.v2":
                if comparison.get("requested_node_budget") != contract.get("nodes"):
                    errors.append(f"{prefix}.comparison.requested_node_budget")
                if comparison.get("same_requested_budget") is not True:
                    errors.append(f"{prefix}.comparison.same_requested_budget")
            for key in ("bestmove_matches_played", "warm_bestmove_changed"):
                if not isinstance(comparison.get(key), bool):
                    errors.append(f"{prefix}.comparison.{key}")
            for key in ("warm_score_delta_cp", "actual_score_delta_cp"):
                if not isinstance(comparison.get(key), int):
                    errors.append(f"{prefix}.comparison.{key}")
    return errors


def main(argv=None) -> int:
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path)
    parser.add_argument("--verify-artifacts", action="store_true")
    parser.add_argument("--root", type=Path, default=Path.cwd())
    args = parser.parse_args(argv)
    path = args.input
    try:
        document = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        print(f"invalid root review: {exc}", file=sys.stderr)
        return 2
    errors = validate(document, root=args.root, verify_artifacts=args.verify_artifacts)
    if errors:
        print("invalid root review: " + ", ".join(errors), file=sys.stderr)
        return 1
    print(f"valid root review: {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
