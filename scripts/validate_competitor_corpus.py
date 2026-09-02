#!/usr/bin/env python3
"""Validate the checked-in Phase 0.5 competitor parity corpus.

This is intentionally a structural check. It does not import or execute an
external engine and therefore cannot establish parity or performance.
"""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CORPUS = ROOT / "scripts" / "competitor_parity_corpus.json"
MOVE_RE = re.compile(r"^(?:[1-9][a-i][1-9][a-i]\+?|[PLNSGBR]\*[1-9][a-i])$")
HAND_RE = re.compile(r"^(?:-|(?:(?:[1-9]\d*)?[PLNSGBR])+)$")
CHECKS = {
    "sfen_roundtrip",
    "do_undo",
    "legal_moves",
    "perft",
    "promotion",
    "drops",
    "check_detection",
    "repetition",
}


def validate_sfen(sfen: object, case_id: str) -> None:
    if not isinstance(sfen, str):
        raise ValueError(f"{case_id}: sfen must be a string")
    fields = sfen.split()
    if len(fields) != 4:
        raise ValueError(f"{case_id}: sfen must have four fields")
    rows = fields[0].split("/")
    if len(rows) != 9:
        raise ValueError(f"{case_id}: board must have nine ranks")
    for rank in rows:
        width = 0
        index = 0
        while index < len(rank):
            char = rank[index]
            if char.isdigit():
                if char == "0":
                    raise ValueError(f"{case_id}: board contains zero-width digit")
                width += int(char)
            elif char == "+":
                if index + 1 >= len(rank) or rank[index + 1] not in "plnsgbrkPLNSGBRK":
                    raise ValueError(f"{case_id}: promotion marker is not followed by a piece")
                width += 1
                index += 1
            elif char in "plnsgbrkPLNSGBRK":
                width += 1
            else:
                raise ValueError(f"{case_id}: invalid board character {char!r}")
            index += 1
        if width != 9:
            raise ValueError(f"{case_id}: rank width is {width}, expected 9")
    if fields[1] not in {"b", "w"}:
        raise ValueError(f"{case_id}: side-to-move must be b or w")
    if not HAND_RE.fullmatch(fields[2]):
        raise ValueError(f"{case_id}: invalid hand field")
    if fields[3] != "1" and not fields[3].isdigit():
        raise ValueError(f"{case_id}: move number must be numeric")


def validate_corpus(path: Path) -> int:
    document = json.loads(path.read_text(encoding="utf-8"))
    if document.get("schema") != "sekirei.competitor-parity.v1":
        raise ValueError("unexpected corpus schema")
    cases = document.get("cases")
    if not isinstance(cases, list) or not cases:
        raise ValueError("cases must be a non-empty list")

    ids: set[str] = set()
    for case in cases:
        case_id = case.get("id")
        if not isinstance(case_id, str) or not case_id:
            raise ValueError("every case needs a non-empty id")
        if case_id in ids:
            raise ValueError(f"duplicate case id: {case_id}")
        ids.add(case_id)
        validate_sfen(case.get("sfen"), case_id)
        moves = case.get("moves", [])
        if not isinstance(moves, list) or any(
            not isinstance(move, str) or not MOVE_RE.fullmatch(move) for move in moves
        ):
            raise ValueError(f"{case_id}: invalid USI move in moves")
        checks = case.get("checks")
        if not isinstance(checks, list) or not checks or any(check not in CHECKS for check in checks):
            raise ValueError(f"{case_id}: invalid checks list")
        perft = case.get("perft", {})
        if not isinstance(perft, dict) or any(
            not str(depth).isdigit() or not isinstance(count, int) or count < 1
            for depth, count in perft.items()
        ):
            raise ValueError(f"{case_id}: invalid perft expectations")

    return len(cases)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("corpus", nargs="?", type=Path, default=DEFAULT_CORPUS)
    args = parser.parse_args()
    count = validate_corpus(args.corpus)
    print(f"competitor corpus OK: cases={count}")


if __name__ == "__main__":
    main()
