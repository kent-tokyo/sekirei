#!/usr/bin/env python3
"""Generate a new score-blind corpus using only Sekirei legal moves and rule facts.

This is diagnostic input, never training data or an independent strength holdout.
The seeded trajectories start from startpos and never read training positions.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import random
import subprocess
from pathlib import Path

from nps_threads import sha256_file

START = "lnsgkgsnl/1r5b1/ppppppppp/9/9/9/PPPPPPPPP/1B5R1/LNSGKGSNL b - 1"


def decode(sfen: str):
    layout, side, hands, number = sfen.split()
    board = []
    for rank in layout.split("/"):
        row, promoted = [], False
        for char in rank:
            if char.isdigit():
                row.extend([None] * int(char))
            elif char == "+":
                promoted = True
            else:
                row.append(("+" if promoted else "") + char)
                promoted = False
        if len(row) != 9:
            raise ValueError("invalid rank width")
        board.extend(row)
    reserves, count = {}, ""
    for char in hands:
        if char.isdigit():
            count += char
        elif char != "-":
            reserves[char] = int(count or "1")
            count = ""
    return board, side, reserves, int(number)


def encode(board, side, hands, number):
    ranks = []
    for offset in range(0, 81, 9):
        rank, empty = "", 0
        for piece in board[offset:offset + 9]:
            if piece is None:
                empty += 1
            else:
                rank += (str(empty) if empty else "") + piece
                empty = 0
        ranks.append(rank + (str(empty) if empty else ""))
    reserve = "".join((str(hands[piece]) if hands[piece] > 1 else "") + piece
                      for piece in "RBGSNLPrbgsnlp" if hands.get(piece, 0))
    return "/".join(ranks) + f" {side} {reserve or '-'} {number}"


def square(text):
    return (ord(text[1]) - ord("a")) * 9 + 9 - int(text[0])


def apply(sfen, move):
    board, side, hands, number = decode(sfen)
    target = square(move[2:4])
    if move[1] == "*":
        piece = move[0] if side == "b" else move[0].lower()
        if board[target] is not None or hands.get(piece, 0) == 0:
            raise ValueError("illegal generated drop")
        hands[piece] -= 1
    else:
        origin = square(move[:2])
        piece = board[origin]
        if piece is None:
            raise ValueError("generated move has empty origin")
        board[origin] = None
        captured = board[target]
        if captured:
            reserve = captured[-1].upper() if side == "b" else captured[-1].lower()
            hands[reserve] = hands.get(reserve, 0) + 1
        if move.endswith("+"):
            piece = "+" + piece
    board[target] = piece
    return encode(board, "w" if side == "b" else "b", hands, number + 1)


def probe(binary: Path, sfens: list[str]) -> list[str]:
    result = subprocess.run([str(binary.resolve())], input="\n".join(sfens) + "\n",
                            text=True, capture_output=True, check=True, timeout=60)
    lines = result.stdout.splitlines()
    if len(lines) != len(sfens):
        raise ValueError("probe result count differs from requested SFEN count")
    return lines


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--legal-moves", type=Path, required=True)
    parser.add_argument("--rule-probe", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--seed", type=int, default=2026101401)
    parser.add_argument("--count-per-category", type=int, default=6)
    parser.add_argument("--trajectories", type=int, default=32)
    parser.add_argument("--max-plies", type=int, default=96)
    parser.add_argument("--exclude", type=Path, action="append", default=[],
                        help="prior diagnostic/gate SFEN file excluded by board+side+hands identity")
    args = parser.parse_args()
    if min(args.count_per_category, args.trajectories, args.max_plies) < 1:
        parser.error("sizes must be positive")
    rng = random.Random(args.seed)
    excluded = set()
    for path in args.exclude:
        for line in path.read_text().splitlines():
            line = line.strip().removeprefix("sfen ")
            if line and not line.startswith("#"):
                if len(line.split()) != 4:
                    raise ValueError("exclusion files require full SFEN records")
                excluded.add(" ".join(line.split()[:3]))
    selected = {"quiet": [], "tactical": []}
    seen = set(excluded)
    for game in range(args.trajectories):
        sfen, moves = START, []
        selected_in_trajectory = set()
        for ply in range(args.max_plies):
            legal = probe(args.legal_moves, [sfen])[0].split()
            if not legal:
                break
            move = legal[rng.randrange(len(legal))]
            sfen = apply(sfen, move)
            moves.append(move)
            if ply + 1 < 24 or (ply + 1) % 8 != game % 2:
                continue
            identity = " ".join(sfen.split()[:3])
            if identity in seen:
                continue
            facts = dict(field.split("=", 1) for field in probe(args.rule_probe, [sfen])[0].split("\t"))
            if facts["in_check"] == "true" or facts["mate_in_one"] == "true" or int(facts["legal_moves"]) < 2:
                continue
            captures, checks = int(facts["capture_moves"]), int(facts["checking_moves"])
            category = "quiet" if captures <= 2 and checks <= 2 else "tactical" if captures >= 4 or checks >= 3 else None
            if category is None or category in selected_in_trajectory or len(selected[category]) >= args.count_per_category:
                continue
            seen.add(identity)
            selected_in_trajectory.add(category)
            selected[category].append({"sfen": sfen, "category": category, "trajectory": game,
                                       "ply": ply + 1, "moves": list(moves), "rule_facts": facts})
        if all(len(group) == args.count_per_category for group in selected.values()):
            break
    if any(len(group) != args.count_per_category for group in selected.values()):
        raise ValueError("generation cap reached before both category quotas; no partial corpus written")
    rows = [row for pair in zip(selected["quiet"], selected["tactical"], strict=True) for row in pair]
    payload = ("\n".join(row["sfen"] for row in rows) + "\n").encode()
    manifest = {"schema": "sekirei.score-blind-search-diagnostic.v1", "seed": args.seed,
                "generator_sha256": sha256_file(Path(__file__)),
                "legal_moves_sha256": sha256_file(args.legal_moves), "rule_probe_sha256": sha256_file(args.rule_probe),
                "corpus_sha256": hashlib.sha256(payload).hexdigest(),
                "selection": {"minimum_ply": 24, "stride": 8, "sample_phase": "trajectory index modulo2", "max_per_category_per_trajectory": 1, "count_per_category": args.count_per_category,
                              "quiet": "capture_moves<=2 and checking_moves<=2", "tactical": "capture_moves>=4 or checking_moves>=3",
                              "exclude_in_check": True, "exclude_mate_in_one": True},
                "excluded_files": [{"path": str(path.resolve()), "sha256": sha256_file(path)} for path in args.exclude],
                "training_corpus_read": False, "external_evaluator": False, "diagnostic_only": True, "positions": rows}
    with args.output.open("xb") as output:
        output.write(payload)
    with args.output.with_suffix(".manifest.json").open("x") as output:
        json.dump(manifest, output, indent=2)
        output.write("\n")
    print(json.dumps({"positions": len(rows), "sha256": manifest["corpus_sha256"]}))


if __name__ == "__main__":
    main()
