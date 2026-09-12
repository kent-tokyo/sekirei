#!/usr/bin/env python3
"""Validate the fixed SP0 do/undo sequence contract."""

import json
import hashlib
import re
import sys
from collections import Counter
from pathlib import Path

DEFAULT_PATH = Path(__file__).resolve().parent / "fixtures/sequence_contract_v1.json"
SFEN = re.compile(r"^[^ ]+ [bw] [^ ]+ [1-9][0-9]*$")
MOVE = re.compile(r"^(?:[1-9][a-i][1-9][a-i]\+?|[PLNSGBR]\*[1-9][a-i])$")
CATEGORIES = (
    "quiet", "capture", "promotion", "capture-promotion", "drop",
    "king move", "check evasion", "mixed",
)


def canonical_sha256(document: dict) -> str:
    canonical = json.dumps(document, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def validate(path: Path = DEFAULT_PATH) -> int:
    document = json.loads(path.read_text(encoding="utf-8"))
    if document.get("schema") != "sekirei.sequence-contract.v1" or document.get("version") != 1:
        raise ValueError("unexpected sequence contract schema or version")
    if document.get("seed") != 0 or not isinstance(document.get("split_policy"), str):
        raise ValueError("sequence contract must declare seed=0 and a split policy")
    series = document.get("series")
    if not isinstance(series, list) or len(series) != 16:
        raise ValueError("series must contain exactly 16 records")
    ids = set()
    categories = Counter()
    sides = Counter()
    splits = Counter()
    for item in series:
        if not isinstance(item, dict) or not isinstance(item.get("id"), str) or item["id"] in ids:
            raise ValueError("series IDs must be non-empty and unique")
        if item.get("category") not in CATEGORIES:
            raise ValueError(f"{item['id']}: invalid category")
        if item.get("side") not in {"b", "w"}:
            raise ValueError(f"{item['id']}: side must be b or w")
        if item.get("split") not in {"tuning", "hold-out"}:
            raise ValueError(f"{item['id']}: split must be tuning or hold-out")
        if not isinstance(item.get("sfen"), str) or not SFEN.fullmatch(item["sfen"]):
            raise ValueError(f"{item['id']}: invalid SFEN envelope")
        sequence = item.get("sequence")
        if not isinstance(sequence, list) or len(sequence) not in {1, 6} or not all(
            isinstance(move, str) and MOVE.fullmatch(move) for move in sequence
        ):
            raise ValueError(f"{item['id']}: sequence must contain 1 or 6 valid USI moves")
        ids.add(item["id"])
        categories[item["category"]] += 1
        sides[item["side"]] += 1
        splits[item["split"]] += 1
    if categories != Counter({category: 2 for category in CATEGORIES}):
        raise ValueError(f"category counts differ: {categories}")
    if sides != Counter({"b": 8, "w": 8}):
        raise ValueError(f"side counts differ: {sides}")
    if splits != Counter({"tuning": 8, "hold-out": 8}):
        raise ValueError(f"split counts differ: {splits}")
    return len(series)


def main() -> int:
    path = Path(sys.argv[1]) if len(sys.argv) == 2 else DEFAULT_PATH
    try:
        count = validate(path)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"sequence contract invalid: {error}", file=sys.stderr)
        return 1
    print(f"sequence contract OK: series={count}, categories={len(CATEGORIES)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
