#!/usr/bin/env python3
"""Validate the internal Floodgate review-manifest evidence contract."""
from __future__ import annotations

import json
import re
import sys
from pathlib import Path


HEX64 = re.compile(r"^[0-9a-f]{64}$")
RAW_STATUSES = {"verified", "invalid", "unknown", "inferred"}


def validate(document: dict) -> list[str]:
    errors = []
    if document.get("schema") != "sekirei.floodgate-review-manifest.v1":
        errors.append("schema")
    if document.get("diagnostic_only") is not True:
        errors.append("diagnostic_only")
    pairs = document.get("pairs")
    if not isinstance(pairs, list):
        return errors + ["pairs"]
    counts = {"verified": 0, "invalid": 0}
    for index, pair in enumerate(pairs):
        prefix = f"pairs[{index}]"
        if not isinstance(pair, dict):
            errors.append(prefix)
            continue
        if not isinstance(pair.get("id"), str) or not pair["id"]:
            errors.append(f"{prefix}.id")
        if pair.get("status") not in {"verified", "invalid"}:
            errors.append(f"{prefix}.status")
        else:
            counts[pair["status"]] += 1
        for artifact_name in ("csa", "analysis"):
            artifact = pair.get(artifact_name)
            if not isinstance(artifact, dict):
                errors.append(f"{prefix}.{artifact_name}")
                continue
            if not isinstance(artifact.get("path"), str) or not artifact["path"]:
                errors.append(f"{prefix}.{artifact_name}.path")
            if not isinstance(artifact.get("bytes"), int) or artifact["bytes"] < 0:
                errors.append(f"{prefix}.{artifact_name}.bytes")
            if not HEX64.fullmatch(artifact.get("sha256", "")):
                errors.append(f"{prefix}.{artifact_name}.sha256")
        evidence = pair.get("evidence_status")
        if not isinstance(evidence, dict):
            errors.append(f"{prefix}.evidence_status")
        else:
            for key in ("raw_pair", "semantic_replay", "evaluator"):
                if evidence.get(key) not in RAW_STATUSES:
                    errors.append(f"{prefix}.evidence_status.{key}")
            if evidence.get("strength") != "not_permitted":
                errors.append(f"{prefix}.evidence_status.strength")
    summary = document.get("summary")
    if not isinstance(summary, dict):
        errors.append("summary")
    else:
        if summary.get("paired") != len(pairs):
            errors.append("summary.paired")
        if summary.get("verified") != counts["verified"]:
            errors.append("summary.verified")
        if summary.get("invalid") != counts["invalid"]:
            errors.append("summary.invalid")
    evidence_summary = document.get("evidence_summary")
    if not isinstance(evidence_summary, dict):
        errors.append("evidence_summary")
    else:
        if evidence_summary.get("raw_pair_verified") != sum(
            pair.get("evidence_status", {}).get("raw_pair") == "verified" for pair in pairs
        ):
            errors.append("evidence_summary.raw_pair_verified")
        if evidence_summary.get("raw_pair_invalid") != sum(
            pair.get("evidence_status", {}).get("raw_pair") == "invalid" for pair in pairs
        ):
            errors.append("evidence_summary.raw_pair_invalid")
        if evidence_summary.get("strength_claim") != "not_permitted":
            errors.append("evidence_summary.strength_claim")
    return errors


def main(argv=None) -> int:
    args = argv or sys.argv[1:]
    if len(args) != 1:
        print(f"usage: {Path(sys.argv[0]).name} MANIFEST.json", file=sys.stderr)
        return 2
    try:
        document = json.loads(Path(args[0]).read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        print(f"invalid review manifest: {exc}", file=sys.stderr)
        return 2
    errors = validate(document)
    if errors:
        print("invalid review manifest: " + ", ".join(errors), file=sys.stderr)
        return 1
    print(f"valid review manifest: {args[0]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
