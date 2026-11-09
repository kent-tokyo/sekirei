#!/usr/bin/env python3
"""Build small original positions for six shogi strategy hypotheses.

All candidate moves are checked against Sekirei's legal generator. A thematic
description identifies the intended comparison; it is not a best-move label.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
from pathlib import Path

from generate_search_diagnostic import encode, square


SPECS = [
    ("material-vs-king-danger", "https://kifulog.shogi.or.jp/seiryu/2026/10/vs-9048.html",
     {"5i": "K", "5a": "k", "8h": "R", "5h": "G", "5c": "r", "8e": "b"},
     {"5i": "K", "5a": "k", "8h": "R", "5h": "G", "4c": "r", "8e": "b"},
     "-", ["8h8e", "5h5g"], ["8h8e", "5h5g"]),
    ("pawn-drop-threat", "https://www.sho-tore.jp/glossary/",
     {"5i": "K", "5a": "k", "8h": "R", "4a": "g"},
     {"5i": "K", "5a": "k", "8h": "R", "6a": "g"},
     "P", ["P*4c", "P*6c"], ["P*4c", "P*6c"]),
    ("defense-and-escape", "https://isf.shogi.or.jp/event/y2021/entry-368.html",
     {"5i": "K", "5a": "k", "5h": "G", "5c": "r"},
     {"5i": "K", "5a": "k", "5h": "G", "6c": "r"},
     "-", ["5h5g", "5i4i"], ["5h5g", "5i4i"]),
    ("inactive-piece", "https://kifulog.shogi.or.jp/seiryu/2026/10/vs-9048.html",
     {"5i": "K", "5a": "k", "9i": "B", "8h": "P"},
     {"5i": "K", "5a": "k", "9i": "B", "8g": "P"},
     "-", ["8h8g", "5i4i"], ["8g8f", "5i4i"]),
    ("pinned-exchange", "https://isf.shogi.or.jp/event/y2021/entry-367.html",
     {"5i": "K", "1a": "k", "5g": "G", "5c": "r", "6g": "s"},
     {"5i": "K", "1a": "k", "5g": "G", "3c": "r", "6g": "s"},
     "-", ["5g5f", "5i4i"], ["5g6g", "5i4i"]),
    ("check-vs-quiet-threat", "https://isf.shogi.or.jp/event/y2021/entry-368.html",
     {"9i": "K", "5a": "k"},
     {"9i": "K", "5a": "k", "4a": "g"},
     "RP", ["R*5b", "P*4c"], ["R*5b", "P*4c"]),
]


def position(pieces: dict[str, str], hands: str) -> str:
    board: list[str | None] = [None] * 81
    for coordinate, piece in pieces.items():
        board[square(coordinate)] = piece
    reserve = {} if hands == "-" else {piece: 1 for piece in hands}
    return encode(board, "b", reserve, 1)


def build(legal_moves: Path) -> dict:
    cases = []
    for theme, link, primary, control, hands, first, second in SPECS:
        for label, pieces, candidates in (("focus", primary, first), ("contrast", control, second)):
            sfen = position(pieces, hands)
            probe = subprocess.run([str(legal_moves.resolve())], input=sfen + "\n",
                                   text=True, capture_output=True, check=True, timeout=5)
            legal = set(probe.stdout.split())
            if not set(candidates) <= legal:
                raise ValueError(f"{theme}/{label}: candidate outside legal set: {set(candidates) - legal}")
            cases.append({"id": f"{theme}-{label}", "sfen": sfen,
                          "source": {"kind": "original_composed_reduced_material",
                                     "research_link": link, "theme": theme,
                                     "theme_is_hypothesis": True,
                                     "reference_is_oracle": False},
                          "stages": [{"id": "first-move", "kind": "candidate_comparison",
                                      "moves": [], "expected_sfen": sfen,
                                      "candidates": candidates, "reference_moves": []}]})
    identities = [" ".join(case["sfen"].split()[:3]) for case in cases]
    if len(set(identities)) != len(cases):
        raise ValueError("duplicate composed position")
    return {"schema": "sekirei.tactical-line-cases.v1", "diagnostic_only": True,
            "legal_moves_sha256": hashlib.sha256(legal_moves.read_bytes()).hexdigest(),
            "note": "Legal candidates only; theme and best-move value remain unproven.",
            "cases": cases}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--legal-moves", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    document = build(args.legal_moves)
    with args.output.open("x", encoding="utf-8") as output:
        json.dump(document, output, ensure_ascii=False, indent=2)
        output.write("\n")
    print(json.dumps({"cases": len(document["cases"]), "themes": len(SPECS)}))


if __name__ == "__main__":
    main()
