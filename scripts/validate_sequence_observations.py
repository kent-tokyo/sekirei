#!/usr/bin/env python3
"""Validate a recorded SP0 sequence-observation manifest."""

import json
import re
import sys
from pathlib import Path

from validate_sequence_contract import DEFAULT_PATH, canonical_sha256, validate

HASH = re.compile(r"^[0-9a-f]{64}$")
MOVE = re.compile(r"^(?:[1-9][a-i][1-9][a-i]\+?|[PLNSGBR]\*[1-9][a-i])$")


def validate_report(path: Path, contract_path: Path = DEFAULT_PATH) -> int:
    report = json.loads(path.read_text(encoding="utf-8"))
    contract = json.loads(contract_path.read_text(encoding="utf-8"))
    contract_count = validate(contract_path)
    if report.get("schema") != "sekirei.sequence-observations.v1" or report.get("version") != 1:
        raise ValueError("unexpected sequence observations schema or version")
    if report.get("contract_sha256") != canonical_sha256(contract):
        raise ValueError("contract SHA-256 mismatch")
    if not isinstance(report.get("binary_sha256"), str) or not HASH.fullmatch(report["binary_sha256"]):
        raise ValueError("binary SHA-256 must be a lowercase hex digest")
    series = report.get("series")
    if report.get("series_count") != contract_count or not isinstance(series, list):
        raise ValueError("series count does not match the contract")
    expected = {item["id"]: item for item in contract["series"]}
    if len(series) != len(expected) or {item.get("id") for item in series} != set(expected):
        raise ValueError("observation series IDs do not match the contract")
    for item in series:
        contract_item = expected[item["id"]]
        if item.get("sfen") != contract_item["sfen"] or item.get("sequence") != contract_item["sequence"]:
            raise ValueError(f"{item['id']}: contract input changed")
        observation = item.get("observation")
        if not isinstance(observation, dict):
            raise ValueError(f"{item['id']}: observation is required")
        legal_moves = observation.get("legal_moves")
        perft2 = observation.get("perft2")
        divide = observation.get("perft2_divide")
        if not isinstance(legal_moves, int) or legal_moves < 1 or not isinstance(perft2, int) or perft2 < 1:
            raise ValueError(f"{item['id']}: invalid legal move or Perft count")
        if not isinstance(divide, list) or len(divide) != legal_moves:
            raise ValueError(f"{item['id']}: Perft divide length mismatch")
        counts = []
        for entry in divide:
            if not isinstance(entry, str) or ":" not in entry:
                raise ValueError(f"{item['id']}: malformed Perft divide entry")
            move, count = entry.split(":", 1)
            if not MOVE.fullmatch(move) or not count.isdigit() or int(count) < 0:
                raise ValueError(f"{item['id']}: malformed Perft divide value")
            counts.append((move, int(count)))
        if len({move for move, _ in counts}) != len(counts) or sum(count for _, count in counts) != perft2:
            raise ValueError(f"{item['id']}: Perft divide does not match Perft(2)")
    return len(series)


def main() -> int:
    if len(sys.argv) not in {2, 3}:
        print(f"usage: {sys.argv[0]} OBSERVATIONS.json [CONTRACT.json]", file=sys.stderr)
        return 2
    try:
        count = validate_report(Path(sys.argv[1]), Path(sys.argv[2]) if len(sys.argv) == 3 else DEFAULT_PATH)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"sequence observations invalid: {error}", file=sys.stderr)
        return 1
    print(f"sequence observations OK: series={count}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
