#!/usr/bin/env python3
"""Pure SFEN/USI 180-degree rotation with side-to-move exchange."""
from __future__ import annotations


HAND_ORDER = "RBGSNLP"


def _swap_piece(piece: str) -> str:
    if piece.startswith("+"):
        return "+" + piece[1].swapcase()
    return piece.swapcase()


def _expand_rank(rank: str) -> list[str]:
    cells: list[str] = []
    index = 0
    while index < len(rank):
        char = rank[index]
        if char.isdigit():
            cells.extend(["1"] * int(char))
            index += 1
        elif char == "+":
            if index + 1 >= len(rank) or rank[index + 1] not in "PLNSGBRplnsgbr":
                raise ValueError("invalid promoted piece")
            cells.append("+" + rank[index + 1])
            index += 2
        elif char in "PLNSGBRplnsgbrKk":
            cells.append(char)
            index += 1
        else:
            raise ValueError(f"invalid board character: {char}")
    if len(cells) != 9:
        raise ValueError("rank must contain nine squares")
    return cells


def _compress_rank(cells: list[str]) -> str:
    result: list[str] = []
    empty = 0
    for cell in cells:
        if cell == "1":
            empty += 1
        else:
            if empty:
                result.append(str(empty))
                empty = 0
            result.append(cell)
    if empty:
        result.append(str(empty))
    return "".join(result)


def _rotate_board(board: str) -> str:
    ranks = [_expand_rank(rank) for rank in board.split("/")]
    if len(ranks) != 9:
        raise ValueError("board must contain nine ranks")
    return "/".join(_compress_rank([_swap_piece(cell) if cell != "1" else "1"
                                     for cell in reversed(rank)])
                     for rank in reversed(ranks))


def rotate_sfen(sfen: str) -> str:
    parts = sfen.split()
    if len(parts) not in {3, 4} or parts[1] not in {"b", "w"}:
        raise ValueError("expected SFEN with board and side to move")
    board, side, hand = parts[:3]
    move_number = parts[3] if len(parts) == 4 else "1"
    # Transform hands by parsing each side separately, then re-encoding in
    # canonical black-uppercase / white-lowercase SFEN order.
    black = _parse_hand_counts(hand, upper=True)
    white = _parse_hand_counts(hand, upper=False)
    transformed_hand = _format_hand(white, upper=True) + _format_hand(black, upper=False)
    return f"{_rotate_board(board)} {'w' if side == 'b' else 'b'} {transformed_hand or '-'} {move_number}"


def _parse_hand_counts(hand: str, upper: bool) -> dict[str, int]:
    counts = {piece: 0 for piece in HAND_ORDER}
    if hand == "-":
        return counts
    index = 0
    while index < len(hand):
        start = index
        while index < len(hand) and hand[index].isdigit():
            index += 1
        count = int(hand[start:index]) if index > start else 1
        if index >= len(hand) or hand[index].upper() not in HAND_ORDER:
            raise ValueError("invalid hand piece")
        piece = hand[index]
        if (piece.isupper()) != upper:
            index += 1
            continue
        counts[piece.upper()] += count
        index += 1
    return counts


def _format_hand(counts: dict[str, int], upper: bool) -> str:
    return "".join((str(count) if count > 1 else "") + (piece if upper else piece.lower())
                   for piece in HAND_ORDER if (count := counts[piece]))


def rotate_usi(move: str) -> str:
    if len(move) < 4:
        raise ValueError("invalid USI move")
    def square(value: str) -> str:
        if len(value) != 2 or value[0] not in "123456789" or value[1] not in "abcdefghi":
            raise ValueError("invalid USI square")
        return str(10 - int(value[0])) + chr(ord("a") + (ord("i") - ord(value[1])))
    if move[1] == "*":
        prefix, destination, suffix = move[:2], move[2:4], move[4:]
        if prefix[0] not in "PLNSGBR" or prefix[1] != "*":
            raise ValueError("invalid USI drop")
        return prefix + square(destination) + suffix
    origin, destination, suffix = move[:2], move[2:4], move[4:]
    if origin[0] not in "123456789" or origin[1] not in "abcdefghi":
        raise ValueError("invalid USI origin")
    return square(origin) + square(destination) + suffix
