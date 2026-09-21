#!/usr/bin/env python3
"""Validate a versioned, separately licensed NNUE release artifact."""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path


SHA256 = re.compile(r"^[0-9a-f]{64}$")


def digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def validate(card: dict, root: Path) -> list[str]:
    errors: list[str] = []
    if card.get("schema") != "sekirei.nnue-model-card.v1":
        errors.append("schema")
    artifact = card.get("artifact")
    if not isinstance(artifact, str) or not artifact:
        return errors + ["artifact"]
    path = root / artifact
    if not path.is_file():
        return errors + ["artifact.missing"]
    if not SHA256.fullmatch(card.get("sha256", "")):
        errors.append("sha256.format")
    elif digest(path) != card["sha256"]:
        errors.append("sha256.mismatch")
    if card.get("bytes") != path.stat().st_size:
        errors.append("bytes")
    if path.read_bytes()[:8] != b"SEKIRW01":
        errors.append("magic")
    if card.get("architecture") != "A-flat-ps" or card.get("nnue_output") != "absolute":
        errors.append("format")
    if card.get("license") != "CC-BY-4.0" or not isinstance(card.get("attribution"), str):
        errors.append("license")
    training = card.get("training")
    if not isinstance(training, dict) or not isinstance(training.get("teacher"), dict):
        errors.append("training")
    gate = card.get("strength_gate")
    if not isinstance(gate, dict) or gate.get("verdict") != "PASS" or gate.get("games") != 94:
        errors.append("strength_gate")
    elif not isinstance(gate.get("scope"), str) or "not a Floodgate" not in gate["scope"]:
        errors.append("strength_gate.scope")
    elif not isinstance(gate.get("baseline"), dict) or not SHA256.fullmatch(
        gate["baseline"].get("sha256", "")
    ):
        errors.append("strength_gate.baseline")
    status = card.get("current_status")
    if not isinstance(status, dict):
        errors.append("current_status")
    elif (
        status.get("availability") != "published_optional_checkpoint"
        or status.get("recommendation") != "hold_after_current_material_comparison"
        or not isinstance(status.get("reason"), str)
        or not status["reason"]
    ):
        errors.append("current_status.contract")
    else:
        comparison = status.get("comparison")
        settings = comparison.get("settings") if isinstance(comparison, dict) else None
        if (
            not isinstance(comparison, dict)
            or comparison.get("status") != "complete"
            or comparison.get("pooled") is not False
            or not isinstance(settings, list)
            or [row.get("byoyomi_ms") for row in settings if isinstance(row, dict)]
            != [1000, 5000]
        ):
            errors.append("current_status.comparison")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("card", type=Path)
    parser.add_argument("--root", type=Path, default=Path.cwd())
    args = parser.parse_args()
    try:
        card = json.loads(args.card.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        print(f"invalid NNUE model card: {error}", file=sys.stderr)
        return 2
    errors = validate(card, args.root)
    if errors:
        print("invalid NNUE model card: " + ", ".join(errors), file=sys.stderr)
        return 1
    print(f"valid NNUE release artifact: {card['artifact']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
