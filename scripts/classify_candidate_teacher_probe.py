#!/usr/bin/env python3
"""Classify candidate/teacher probe divergence by SFEN position features."""

from __future__ import annotations

import argparse
import json
import statistics
from collections import defaultdict
from pathlib import Path
from typing import Any


def hand_units(hand: str) -> int:
    if hand == "-":
        return 0
    total = 0
    count = ""
    for char in hand:
        if char.isdigit():
            count += char
        else:
            total += int(count or "1")
            count = ""
    if count:
        raise ValueError(f"hand count without piece: {hand}")
    return total


def features(row: dict[str, Any]) -> dict[str, Any]:
    fields = row["sfen"].split()
    board, side, hand, ply_text = fields[:4]
    pieces = [char for char in board if char.isalpha()]
    return {
        **row,
        "side_to_move": side,
        "ply": int(ply_text),
        "phase": "early" if int(ply_text) < 80 else "middle" if int(ply_text) < 120 else "late",
        "piece_band": "sparse" if len(pieces) <= 12 else "normal" if len(pieces) <= 24 else "crowded",
        "piece_count": len(pieces),
        "hand_band": "none" if hand == "-" else "small" if hand_units(hand) <= 4 else "large",
        "hand_units": hand_units(hand),
        "promotion": "promoted" if "+" in board else "unpromoted",
    }


def load(path: Path) -> dict[str, dict[str, Any]]:
    rows = {}
    for line_number, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not raw.strip():
            continue
        row = json.loads(raw)
        sample_id = row.get("sample_id")
        if not isinstance(sample_id, str) or not sample_id:
            raise ValueError(f"{path}:{line_number}: missing sample_id")
        if sample_id in rows:
            raise ValueError(f"{path}:{line_number}: duplicate sample_id {sample_id}")
        rows[sample_id] = row
    return rows


def summarize(rows: list[dict[str, Any]]) -> dict[str, Any]:
    ok = [row for row in rows if row["candidate"].get("status") == "ok" and row["teacher"].get("status") == "ok"]
    mismatches = [row for row in ok if row["candidate"]["bestmove"] != row["teacher"]["bestmove"]]
    deltas = [row["candidate"]["score"] - row["teacher"]["score"] for row in ok]
    return {
        "positions": len(rows),
        "shared_ok": len(ok),
        "bestmove_mismatch": len(mismatches),
        "bestmove_mismatch_rate": len(mismatches) / len(ok) if ok else None,
        "mean_signed_delta_cp": statistics.fmean(deltas) if deltas else None,
        "mean_absolute_delta_cp": statistics.fmean(abs(delta) for delta in deltas) if deltas else None,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("corpus", type=Path)
    parser.add_argument("candidate", type=Path)
    parser.add_argument("teacher", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    corpus = load(args.corpus)
    candidate = load(args.candidate)
    teacher = load(args.teacher)
    if not set(candidate) <= set(corpus) or not set(teacher) <= set(corpus) or set(candidate) != set(teacher):
        raise SystemExit("probe sample IDs are not a common corpus subset")
    rows = []
    for sample_id in candidate:
        corpus_row = corpus[sample_id]
        candidate_row = candidate[sample_id]
        teacher_row = teacher[sample_id]
        rows.append({
            **features(corpus_row),
            "candidate": {
                "status": candidate_row.get("status"),
                "bestmove": candidate_row.get("bestmove"),
                "score": candidate_row.get("lines", [{}])[0].get("score_cp") if candidate_row.get("lines") else None,
            },
            "teacher": {
                "status": teacher_row.get("status"),
                "bestmove": teacher_row.get("bestmove"),
                "score": teacher_row.get("lines", [{}])[0].get("score_cp") if teacher_row.get("lines") else None,
            },
        })
    dimensions = ("phase", "piece_band", "hand_band", "promotion", "side_to_move")
    grouped: dict[str, list[dict[str, Any]]] = {"overall": rows}
    for dimension in dimensions:
        buckets: dict[str, list[dict[str, Any]]] = defaultdict(list)
        for row in rows:
            buckets[str(row[dimension])].append(row)
        for value, bucket in sorted(buckets.items()):
            grouped[f"{dimension}={value}"] = bucket
    report = {
        "schema_version": 1,
        "diagnostic_only": True,
        "corpus_positions": len(corpus),
        "probed_positions": len(rows),
        "groups": {key: summarize(value) for key, value in grouped.items()},
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
