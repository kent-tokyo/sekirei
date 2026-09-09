#!/usr/bin/env python3
"""Validate a frozen strength-gate plan against its local input files."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def validate(manifest: dict[str, object]) -> list[str]:
    errors: list[str] = []
    if manifest.get("schema") != "sekirei.strength-gate-plan.v1":
        errors.append("schema")
    if manifest.get("status") != "planned":
        errors.append("status")
    if manifest.get("strength_claim") is not False:
        errors.append("strength_claim")
    protocol = manifest.get("protocol", {})
    if not isinstance(protocol, dict):
        return ["protocol"]
    for key in ("games_per_position", "byoyomi_ms", "max_games"):
        if not isinstance(protocol.get(key), int) or protocol[key] <= 0:
            errors.append(f"protocol.{key}")
    sprt = protocol.get("sprt", {})
    if not isinstance(sprt, dict) or sprt.get("elo0") != 0 or sprt.get("elo1") != 20:
        errors.append("protocol.sprt")
    options = protocol.get("engine_options", {})
    if options != {"Threads": "1", "SpecTopN": "0", "UseBook": "false", "SearchMode": "Speculative"}:
        errors.append("protocol.engine_options")
    return errors


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    args = parser.parse_args()
    manifest = json.loads(args.manifest.read_text(encoding="utf-8"))
    errors = validate(manifest)
    for name in ("candidate", "baseline", "openings", "calibration"):
        item = manifest.get(name, {})
        path_value = item.get("path") if isinstance(item, dict) else None
        expected = item.get("sha256") if isinstance(item, dict) else None
        if not isinstance(path_value, str) or not isinstance(expected, str):
            errors.append(f"{name}.metadata")
            continue
        path = Path(path_value)
        if not path.is_file():
            errors.append(f"{name}.path")
            continue
        if sha256(path) != expected:
            errors.append(f"{name}.sha256")
        if name == "calibration":
            try:
                calibration = json.loads(path.read_text(encoding="utf-8"))
                if not isinstance(calibration, dict) or validate_calibration(calibration):
                    errors.append("calibration.result")
            except (OSError, json.JSONDecodeError):
                errors.append("calibration.result")
        if name == "openings":
            actual_positions = sum(1 for line in path.read_text(encoding="utf-8").splitlines() if line.strip() and not line.startswith("#"))
            if actual_positions != item.get("positions"):
                errors.append("openings.positions")
    if errors:
        raise SystemExit("invalid strength-gate manifest: " + ", ".join(errors))
    print(f"strength-gate manifest valid: {args.manifest}")


def validate_calibration(data: dict[str, object]) -> list[str]:
    required = ("games", "engine1_wins", "draws", "engine2_wins", "diversity_ratio")
    if any(key not in data for key in required):
        return ["missing fields"]
    games = int(data["games"])
    if games <= 0 or sum(int(data[key]) for key in required[1:4]) != games:
        return ["outcome counts"]
    if float(data["diversity_ratio"]) < 1.0:
        return ["insufficient diversity"]
    return []


if __name__ == "__main__":
    main()
