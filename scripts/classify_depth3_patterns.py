#!/usr/bin/env python3
"""Classify depth-3 evaluator divergences by SFEN and score patterns."""

import argparse
import json
from collections import Counter, defaultdict
from pathlib import Path


def load(path: Path) -> dict[str, dict]:
    return {json.loads(line)["sample_id"]: json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()}


def sign(value: float) -> int:
    return (value > 0) - (value < 0)


def hand_units(hand: str) -> int:
    total, digits = 0, ""
    for char in hand:
        if char.isdigit():
            digits += char
        else:
            total += int(digits or "1")
            digits = ""
    return total


def features(sfen: str) -> dict[str, str]:
    board, side, hand, ply_text = sfen.split()[:4]
    pieces = sum(char.isalpha() for char in board)
    ply = int(ply_text)
    return {
        "phase": "early" if ply < 80 else "middle" if ply < 120 else "late",
        "side": side,
        "promotion": "promoted" if "+" in board else "unpromoted",
        "piece_band": "normal" if pieces <= 24 else "crowded",
        "hand_band": "none" if hand == "-" else "small" if hand_units(hand) <= 4 else "large",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("corpus", type=Path)
    parser.add_argument("candidate", type=Path)
    parser.add_argument("teacher", type=Path)
    parser.add_argument("material", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    corpus, candidate, teacher, material = (load(path) for path in (args.corpus, args.candidate, args.teacher, args.material))
    rows = []
    for sample_id in sorted(candidate.keys() & teacher.keys() & material.keys()):
        c, t, m = candidate[sample_id], teacher[sample_id], material[sample_id]
        if not all(row.get("status") == "ok" for row in (c, t, m)) or c["bestmove"] == t["bestmove"]:
            continue
        cs, ts, ms = float(c["lines"][0]["score_cp"]), float(t["lines"][0]["score_cp"]), float(m["lines"][0]["score_cp"])
        delta = abs(cs - ts)
        move_bucket = "candidate_material" if c["bestmove"] == m["bestmove"] else "teacher_material" if t["bestmove"] == m["bestmove"] else "all_distinct"
        score_bucket = "small" if delta <= 100 else "medium" if delta <= 300 else "large"
        sign_bucket = "opposite_nonzero" if sign(cs) * sign(ts) == -1 else "same_sign_or_zero"
        rows.append({**features(corpus[sample_id]["sfen"]), "sample_id": sample_id, "move_bucket": move_bucket, "score_bucket": score_bucket, "sign_bucket": sign_bucket})
    groups = defaultdict(list)
    groups["overall"] = rows
    for dimension in ("phase", "side", "promotion", "piece_band", "hand_band", "move_bucket", "score_bucket", "sign_bucket"):
        buckets = defaultdict(list)
        for row in rows:
            buckets[row[dimension]].append(row)
        groups.update({f"{dimension}={key}": value for key, value in buckets.items()})
    report = {
        "schema_version": 1,
        "diagnostic_only": True,
        "divergent_positions": len(rows),
        "groups": {key: {"positions": len(value), "phase_counts": dict(Counter(row["phase"] for row in value)), "side_counts": dict(Counter(row["side"] for row in value)), "move_buckets": dict(Counter(row["move_bucket"] for row in value)), "score_buckets": dict(Counter(row["score_bucket"] for row in value)), "sign_buckets": dict(Counter(row["sign_bucket"] for row in value))} for key, value in sorted(groups.items())},
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
