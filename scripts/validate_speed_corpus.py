#!/usr/bin/env python3
"""Validate the checked-in rsshogi speed-corpus contract."""

import json
import hashlib
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CORPUS = ROOT / "scripts/fixtures/speed_corpus_v1.json"
CATEGORIES = ("opening", "midgame", "hands", "check", "capture", "promotion", "endgame", "tactical")
SFEN = re.compile(r"^[^ ]+ [bw] [^ ]+ [1-9][0-9]*$")
MOVE = re.compile(r"^(?:[1-9][a-i][1-9][a-i]\+?|[PLNSGBR]\*[1-9][a-i])$")
PIECES = set("pnslgbrkPNSLGBRK")
HAND = re.compile(r"^(?:-|(?:(?:[1-9][0-9]*)?[PLNSGBRplnsgbr])*)$")


def _validate_board(board: str, case_id: str) -> None:
    ranks = board.split("/")
    if len(ranks) != 9:
        raise ValueError(f"{case_id}: board must have 9 ranks")
    for rank_index, rank in enumerate(ranks, 1):
        width = 0
        promoted = False
        for char in rank:
            if char.isdigit():
                if char == "0":
                    raise ValueError(f"{case_id}: zero in rank {rank_index}")
                width += int(char)
            elif char == "+":
                if promoted:
                    raise ValueError(f"{case_id}: repeated promotion marker")
                promoted = True
            elif char in PIECES:
                width += 1
                promoted = False
            else:
                raise ValueError(f"{case_id}: invalid board character {char!r}")
        if promoted or width != 9:
            raise ValueError(f"{case_id}: rank {rank_index} has width {width}")


def validate_document(doc: dict) -> int:
    if doc.get("schema") != "sekirei.speed-corpus.v1" or doc.get("version") != 1:
        raise ValueError("unexpected speed corpus schema or version")
    cases = doc.get("cases")
    if not isinstance(cases, list) or len(cases) != 128:
        raise ValueError("cases must contain exactly 128 records")
    required_case_ids = doc.get("required_case_ids")
    if not isinstance(required_case_ids, list) or len(required_case_ids) != 128:
        raise ValueError("required_case_ids must contain exactly 128 records")
    corpus_sha256 = doc.get("corpus_sha256")
    if not isinstance(corpus_sha256, str) or len(corpus_sha256) != 64:
        raise ValueError("corpus_sha256 must be a SHA-256 hex digest")
    if not all(isinstance(case_id, str) and case_id for case_id in required_case_ids):
        raise ValueError("required_case_ids must contain non-empty strings")
    if len(set(required_case_ids)) != len(required_case_ids):
        raise ValueError("required_case_ids must be unique")
    ids: set[str] = set()
    sfens: set[str] = set()
    counts = {category: 0 for category in CATEGORIES}
    for case in cases:
        if not isinstance(case, dict):
            raise ValueError("case must be an object")
        case_id, category, sfen = case.get("id"), case.get("category"), case.get("sfen")
        if not isinstance(case_id, str) or not case_id or case_id in ids:
            raise ValueError(f"invalid or duplicate case id: {case_id!r}")
        if category not in counts:
            raise ValueError(f"{case_id}: unknown category {category!r}")
        if not isinstance(sfen, str) or sfen in sfens or not SFEN.fullmatch(sfen):
            raise ValueError(f"{case_id}: invalid or duplicate SFEN")
        board, side, hands, _ply = sfen.split()
        _validate_board(board, case_id)
        if not HAND.fullmatch(hands):
            raise ValueError(f"{case_id}: invalid hand field")
        if not isinstance(case.get("source"), str) or not case["source"]:
            raise ValueError(f"{case_id}: source is required")
        if case.get("derivation") not in {
            "base", "file-mirror", "rank-mirror", "base-opposite-side",
            "file-mirror-opposite-side", "rank-mirror-opposite-side"
        }:
            raise ValueError(f"{case_id}: invalid derivation")
        expected = case.get("expected")
        if not isinstance(expected, dict) or any(
            not isinstance(expected.get(key), int) or expected[key] < 1
            for key in ("legal_moves", "perft2")
        ):
            raise ValueError(f"{case_id}: expected legal_moves/perft2 are required")
        legal_moves_usi = expected.get("legal_moves_usi")
        if not isinstance(legal_moves_usi, list) or len(legal_moves_usi) != expected["legal_moves"]:
            raise ValueError(f"{case_id}: expected legal move set is required")
        if legal_moves_usi != sorted(set(legal_moves_usi)):
            raise ValueError(f"{case_id}: expected legal move set must be sorted and unique")
        if not all(isinstance(move, str) and MOVE.fullmatch(move) for move in legal_moves_usi):
            raise ValueError(f"{case_id}: expected legal move set contains invalid USI")
        divide = expected.get("perft2_divide")
        if not isinstance(divide, list) or len(divide) != expected["legal_moves"]:
            raise ValueError(f"{case_id}: expected Perft(2) divide is required")
        if len({item.split(":", 1)[0] for item in divide}) != len(divide):
            raise ValueError(f"{case_id}: expected Perft(2) divide moves must be unique")
        if divide != sorted(divide, key=lambda item: item.split(":", 1)[0]):
            raise ValueError(f"{case_id}: expected Perft(2) divide must be sorted and unique")
        if not all(
            isinstance(item, str)
            and ":" in item
            and MOVE.fullmatch(item.split(":", 1)[0])
            and item.split(":", 1)[1].isdigit()
            and int(item.split(":", 1)[1]) >= 0
            for item in divide
        ):
            raise ValueError(f"{case_id}: expected Perft(2) divide contains invalid data")
        if sum(int(item.split(":", 1)[1]) for item in divide) != expected["perft2"]:
            raise ValueError(f"{case_id}: Perft(2) divide does not sum to expected total")
        sequence = case.get("sequence")
        if not isinstance(sequence, list) or not sequence or not all(
            isinstance(move, str) and MOVE.fullmatch(move) for move in sequence
        ):
            raise ValueError(f"{case_id}: sequence must be a non-empty string list")
        ids.add(case_id)
        sfens.add(sfen)
        counts[category] += 1
    actual_case_ids = [case["id"] for case in cases]
    if actual_case_ids != required_case_ids:
        raise ValueError("case IDs differ from required_case_ids")
    if counts != {category: 16 for category in CATEGORIES}:
        raise ValueError(f"category counts differ: {counts}")
    canonical = json.dumps(cases, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
    if hashlib.sha256(canonical.encode("utf-8")).hexdigest() != corpus_sha256:
        raise ValueError("corpus_sha256 does not match cases")
    return len(cases)


def validate(path: Path = DEFAULT_CORPUS) -> int:
    return validate_document(json.loads(path.read_text(encoding="utf-8")))


def main() -> int:
    path = Path(sys.argv[1]) if len(sys.argv) == 2 else DEFAULT_CORPUS
    try:
        count = validate(path)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"speed corpus invalid: {error}", file=sys.stderr)
        return 1
    print(f"speed corpus OK: cases={count}, categories={len(CATEGORIES)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
