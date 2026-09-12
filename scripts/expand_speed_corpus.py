#!/usr/bin/env python3
"""Expand the speed smoke corpus with verified opposite-side positions."""

import argparse
import hashlib
import json
import subprocess
from pathlib import Path


def mirror_rank(rank: str) -> str:
    cells = []
    index = 0
    while index < len(rank):
        char = rank[index]
        if char.isdigit():
            cells.extend("." for _ in range(int(char)))
        elif char == "+":
            index += 1
            cells.append("+" + rank[index])
        else:
            cells.append(char)
        index += 1
    mirrored = list(reversed(cells))
    result = []
    empty = 0
    for cell in mirrored:
        if cell == ".":
            empty += 1
        else:
            if empty:
                result.append(str(empty))
                empty = 0
            result.append(cell)
    if empty:
        result.append(str(empty))
    return "".join(result)


def mirror_sfen(sfen: str) -> str:
    board, side, hand, ply = sfen.split(" ")
    return f"{'/'.join(mirror_rank(rank) for rank in board.split('/'))} {side} {hand} {ply}"


def mirror_sfen_vertical(sfen: str) -> str:
    board, side, hand, ply = sfen.split(" ")
    ranks = board.split("/")
    return f"{'/'.join(reversed(ranks))} {side} {hand} {ply}"


def mirror_usi(move: str) -> str:
    if "*" in move:
        piece, destination = move.split("*", 1)
        return f"{piece}*{10 - int(destination[0])}{destination[1]}"
    source, destination = move[:2], move[2:4]
    suffix = move[4:]
    return f"{10 - int(source[0])}{source[1]}{10 - int(destination[0])}{destination[1]}{suffix}"


def mirror_usi_vertical(move: str) -> str:
    def mirror_rank(rank: str) -> str:
        return chr(ord("a") + 8 - (ord(rank) - ord("a")))

    if "*" in move:
        piece, destination = move.split("*", 1)
        return f"{piece}*{destination[0]}{mirror_rank(destination[1])}"
    source, destination = move[:2], move[2:4]
    suffix = move[4:]
    return f"{source[0]}{mirror_rank(source[1])}{destination[0]}{mirror_rank(destination[1])}{suffix}"


def details(binary: Path, sfen: str) -> tuple[dict[str, int], list[str], list[str]]:
    output = subprocess.check_output([str(binary), "--check-sfen", sfen], text=True)
    line = next(line for line in output.splitlines() if line.startswith("sfen_details="))
    counts = {key: int(value) for key, value in (
        item.split(":", 1) for item in line.removeprefix("sfen_details=").split(";")
    )}
    moves = next(line for line in output.splitlines() if line.startswith("sfen_moves="))
    divide = next(line for line in output.splitlines() if line.startswith("sfen_perft2_divide="))
    return counts, moves.removeprefix("sfen_moves=").split(","), divide.removeprefix("sfen_perft2_divide=").split(",")


def cases_hash(cases: list[dict]) -> str:
    canonical = json.dumps(cases, ensure_ascii=False, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def expand(path: Path, binary: Path) -> int:
    document = json.loads(path.read_text(encoding="utf-8"))
    original = [
        case for case in document["cases"]
        if case.get("derivation", "base") == "base"
        and not case["id"].endswith("-opposite")
    ]
    additions = []
    for case in original:
        expected, moves, divide = details(binary, case["sfen"])
        case["expected"] = expected | {"legal_moves_usi": moves, "perft2_divide": divide}
        fields = case["sfen"].split(" ")
        fields[1] = "w" if fields[1] == "b" else "b"
        opposite = dict(case)
        opposite["id"] = f"{case['id']}-opposite"
        opposite["sfen"] = " ".join(fields)
        opposite["source"] = f"{case['source']}-opposite-side"
        expected, moves, divide = details(binary, opposite["sfen"])
        opposite["expected"] = expected | {"legal_moves_usi": moves, "perft2_divide": divide}
        additions.append(opposite)
    document["purpose"] = (
        "SP0 128-position smoke corpus: 8 categories x 8 derived positions x 2 sides; "
        "four file-mirrored bases supplement four original bases; legal-move and "
        "cross-library preflight is required before timing"
    )
    all_bases = []
    for case in original:
        case["derivation"] = "base"
        all_bases.append(case)
        mirrored = dict(case)
        mirrored["id"] = f"{case['id']}-mirror"
        mirrored["sfen"] = mirror_sfen(case["sfen"])
        mirrored["source"] = f"{case['source']}-file-mirror"
        mirrored["derivation"] = "file-mirror"
        mirrored["sequence"] = [mirror_usi(move) for move in case["sequence"]]
        if mirrored["sfen"] == case["sfen"]:
            mirrored["sfen"] = mirror_sfen_vertical(case["sfen"])
            mirrored["source"] = f"{case['source']}-rank-mirror"
            mirrored["derivation"] = "rank-mirror"
            mirrored["sequence"] = [mirror_usi_vertical(move) for move in case["sequence"]]
        all_bases.append(mirrored)
    additions = []
    for case in all_bases:
        expected, moves, divide = details(binary, case["sfen"])
        case["expected"] = expected | {"legal_moves_usi": moves, "perft2_divide": divide}
        opposite = dict(case)
        opposite["id"] = f"{case['id']}-opposite"
        fields = case["sfen"].split(" ")
        fields[1] = "w" if fields[1] == "b" else "b"
        opposite["sfen"] = " ".join(fields)
        opposite["source"] = f"{case['source']}-opposite-side"
        opposite["derivation"] = f"{case['derivation']}-opposite-side"
        expected, moves, divide = details(binary, opposite["sfen"])
        opposite["expected"] = expected | {"legal_moves_usi": moves, "perft2_divide": divide}
        additions.append(opposite)
    document["cases"] = [case for pair in zip(all_bases, additions) for case in pair]
    document["required_case_ids"] = [case["id"] for case in document["cases"]]
    document["corpus_sha256"] = cases_hash(document["cases"])
    path.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return len(document["cases"])


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--corpus", type=Path, required=True)
    args = parser.parse_args()
    count = expand(args.corpus, args.binary.resolve(strict=True))
    print(f"expanded speed corpus: cases={count}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
