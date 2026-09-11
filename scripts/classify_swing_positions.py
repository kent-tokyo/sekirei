#!/usr/bin/env python3
"""Classify negative, non-terminal analysis swings from a batch report."""
import argparse
import json
import re
from pathlib import Path

PIECE_VALUES = {"P": 100, "L": 300, "N": 300, "S": 400, "G": 500, "B": 800, "R": 1000, "K": 0}
PROMOTED_VALUES = {"P": 500, "L": 500, "N": 500, "S": 500, "B": 900, "R": 1200}
MOVE = re.compile(r"^[+-][0-9]{4}[A-Z]{2}$")


def csa_moves(path):
    try:
        path = Path(path)
        return [line.split(",", 1)[0] for line in path.read_text(encoding="utf-8").splitlines() if MOVE.fullmatch(line.split(",", 1)[0])]
    except OSError:
        return []


def material(sfen, our_color):
    fields = sfen.split()
    board = fields[0]
    hands = fields[2] if len(fields) > 2 else "-"
    totals = {"black": 0, "white": 0}
    promoted = False
    for char in board:
        if char == "+":
            promoted = True
            continue
        if char == "/" or char.isdigit():
            promoted = False
            continue
        color = "black" if char.isupper() else "white"
        piece = char.upper()
        totals[color] += (PROMOTED_VALUES if promoted else PIECE_VALUES).get(piece, 0)
        promoted = False
    if hands != "-":
        count = 0
        for char in hands:
            if char.isdigit():
                count = count * 10 + int(char)
                continue
            color = "black" if char.isupper() else "white"
            totals[color] += count * PIECE_VALUES.get(char.upper(), 0)
            count = 0
        # A count without a following piece is malformed, but do not hide it.
        if count:
            raise ValueError("malformed SFEN hand count")
    signed = totals[our_color] - totals["white" if our_color == "black" else "black"]
    return {"our_material_cp": signed, "black_material_cp": totals["black"], "white_material_cp": totals["white"]}


def classify(batch):
    rows = []
    for report in batch.get("reports", []):
        moves = csa_moves(report.get("csa", ""))
        for record in report.get("records", []):
            if not record.get("swing") or record.get("terminal") or record.get("score_delta_cp", 0) >= 0:
                continue
            fields = record["sfen"].split()
            our_color = "black" if fields[1] == "b" else "white"
            row = {
                "game_id": report.get("game_id"),
                "result": report.get("result"),
                "ply": record.get("ply"),
                "previous_ply": record.get("previous_ply"),
                "phase": record.get("phase", "unknown"),
                "actual_move_csa": record.get("actual_move_csa"),
                "previous_move_csa": moves[record["ply"] - 1] if isinstance(record.get("ply"), int) and 1 <= record["ply"] <= len(moves) else None,
                "two_moves_back_csa": moves[record["ply"] - 2] if isinstance(record.get("ply"), int) and 2 <= record["ply"] <= len(moves) else None,
                "score_delta_cp": record.get("score_delta_cp"),
                "previous_score_cp": record.get("previous_score_cp"),
                "depth": record.get("depth"),
                "elapsed_ms": record.get("elapsed_ms"),
                "our_color": our_color,
                "sfen": record["sfen"],
            }
            row.update(material(record["sfen"], our_color))
            row["material_bucket"] = "ahead" if row["our_material_cp"] > 100 else "behind" if row["our_material_cp"] < -100 else "balanced"
            rows.append(row)
    rows.sort(key=lambda row: row["score_delta_cp"])
    return {"schema": "sekirei.negative-swing-classification.v1", "diagnostic_only": True, "positions": rows}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("batch", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = classify(json.loads(args.batch.read_text(encoding="utf-8")))
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {len(result['positions'])} negative swings")


if __name__ == "__main__":
    main()
