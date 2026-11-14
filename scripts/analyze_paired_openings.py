#!/usr/bin/env python3
"""Describe completed paired openings; never select roots or issue a verdict.

Material is a static piece inventory, not a search/NNUE score. Subgroups are
posthoc diagnostics and cannot be used to relabel or truncate a past trial.
"""
from __future__ import annotations

import argparse
from collections import Counter, defaultdict
import hashlib
import json
from pathlib import Path
import re

VALUES = {"P": 100, "L": 430, "N": 470, "S": 640, "G": 680, "B": 890,
          "R": 1040, "K": 0, "+P": 600, "+L": 600, "+N": 600,
          "+S": 640, "+B": 1150, "+R": 1300}
SCORES = {"candidate_win": 2, "baseline_win": 0, "draw": 1}


def sha(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def material(sfen):
    """Return board+hand material in Black-minus-White centipawns."""
    board, side, hand, ply = sfen.split()
    if side not in ("b", "w") or not ply.isdigit() or int(ply) < 1:
        raise ValueError("invalid SFEN side or move number")
    ranks = board.split("/")
    if len(ranks) != 9:
        raise ValueError("invalid SFEN board height")
    total = 0
    for rank in ranks:
        tokens = re.findall(r"\+?[PLNSGBRKplnsgbrk]|[1-9]", rank)
        if "".join(tokens) != rank or sum(int(t) if t.isdigit() else 1 for t in tokens) != 9:
            raise ValueError("invalid SFEN rank")
        for token in tokens:
            if not token.isdigit():
                if token.upper() not in VALUES:
                    raise ValueError("invalid promoted piece")
                total += VALUES[token.upper()] * (1 if token[-1].isupper() else -1)
    if hand != "-":
        tokens = re.findall(r"([1-9][0-9]*)?([PLNSGBRplnsgbr])", hand)
        if "".join(count + piece for count, piece in tokens) != hand:
            raise ValueError("invalid SFEN hand")
        if len({piece for _, piece in tokens}) != len(tokens):
            raise ValueError("duplicate hand piece")
        total += sum(int(count or 1) * VALUES[piece.upper()] * (1 if piece.isupper() else -1)
                     for count, piece in tokens)
    return total


def material_bucket(value):
    magnitude = abs(value)
    return "0" if not magnitude else "1..300" if magnitude <= 300 else "301..1000" if magnitude <= 1000 else ">1000"


def pair_outcome(first, second):
    if first not in SCORES or second not in SCORES:
        raise ValueError("invalid game result")
    score = SCORES[first] + SCORES[second]
    if score > 2:
        return "candidate_pair_win"
    if score < 2:
        return "candidate_pair_loss"
    return "double_draw" if first == second == "draw" else "split"


def verify_kifu(text, opening, outcome, candidate_colour):
    lines = text.splitlines()
    other = "White" if candidate_colour == "Black" else "Black"
    for side, colour in ((1, candidate_colour), (2, other)):
        rows = [line for line in lines if line.startswith(f"# Engine{side}:")]
        if len(rows) != 1 or not rows[0].endswith(f"({colour})"):
            raise ValueError("kifu colour assignment differs")
    result = {"candidate_win": "Engine1 Win", "baseline_win": "Engine2 Win", "draw": "Draw"}[outcome]
    result_lines = [line for line in lines if line.startswith("# Result:")]
    pattern = r"# Result: Draw(?: \([^\n]*\))?" if outcome == "draw" else re.escape(f"# Result: {result}")
    if len(result_lines) != 1 or re.fullmatch(pattern, result_lines[0]) is None:
        raise ValueError("kifu outcome differs")
    positions = [line.split()[1:] for line in lines if line.startswith("position ")]
    expected = opening.split()
    if expected[:1] == ["position"]:
        expected = expected[1:]
    if len(positions) != 1 or positions[0][:len(expected)] != expected:
        raise ValueError("kifu opening history differs")


def summarize(rows):
    count = len(rows)
    outcomes = Counter(row["pair_outcome"] for row in rows)
    decisive = outcomes["candidate_pair_win"] + outcomes["candidate_pair_loss"]
    split_winners = Counter(row["split_winner_colour"] for row in rows if row["pair_outcome"] == "split")
    predictable = [row for row in rows if row["pair_outcome"] == "split" and row["black_material_cp"] != 0]
    return {"pairs": count, "outcomes": {key: outcomes[key] for key in
            ("candidate_pair_win", "candidate_pair_loss", "split", "double_draw")},
            "decisive_pairs": decisive, "decisive_fraction": decisive / count if count else None,
            "split_winner_colour": dict(split_winners),
            "material_sign_predicts_split_winner": {
                "eligible_pairs": len(predictable),
                "matching_pairs": sum((row["black_material_cp"] > 0) == (row["split_winner_colour"] == "Black")
                                      for row in predictable)}}


def analyze(directory):
    directory = Path(directory).resolve()
    # Do not inspect ongoing trial outcomes for root filtering or tuning.
    decision_path = directory / "decision.json"
    if not decision_path.exists():
        raise ValueError("requires a completed, finalized trial")
    decision = json.loads(decision_path.read_text())
    plan_path = directory / "plan.json"
    plan = json.loads(plan_path.read_text())
    result_path = directory / "match-result.json"
    result = json.loads(result_path.read_text())
    if sha(result_path) != decision["match_result_sha256"]:
        raise ValueError("finalized result receipt differs")
    if result["status"] != "completed" and result["status"] != "inconclusive":
        raise ValueError("requires a complete fixed-cap trial")
    if not result["evidence"]["complete"] or decision["games"] != plan["games_cap"]:
        raise ValueError("trial horizon is incomplete")
    root_path = Path(plan["pins"]["root_manifest"]["path"])
    if sha(plan_path) != decision["plan_sha256"] or sha(root_path) != decision["root_manifest_sha256"]:
        raise ValueError("frozen plan or roots differ")
    events_path = Path(result["evidence"]["outputs"]["raw_match_jsonl"]["path"])
    if sha(events_path) != decision["events_sha256"]:
        raise ValueError("durable game records differ")
    roots = json.loads(root_path.read_text())["positions"]
    events = [json.loads(line) for line in events_path.read_text().splitlines()]
    if len(events) != decision["games"] or len(events) != len(roots) * 2:
        raise ValueError("root/game count differs")
    kifu = result["evidence"]["outputs"]["kifu_manifest"]
    files = {row["name"]: row for row in kifu["files"]}
    if len(files) != len(events) or len(kifu["files"]) != len(events) or kifu["file_count"] != len(events):
        raise ValueError("kifu count differs")
    rows = []
    for index, root in enumerate(roots):
        first, second = events[2 * index:2 * index + 2]
        if any(record["id"] != f"pos{index}_pair0" for record in (first, second)):
            raise ValueError("paired record order differs")
        outcome = pair_outcome(first["result"], second["result"])
        for offset, record in enumerate((first, second)):
            name = f"game{index * 2 + offset + 1:04}.txt"
            path = Path(kifu["directory"]) / name
            if sha(path) != files[name]["sha256"]:
                raise ValueError("kifu hash differs")
            verify_kifu(path.read_text(), root["position"], record["result"], "Black" if offset == 0 else "White")
        value = material(root["sfen"])
        rows.append({"root_index": index, "identity_sha256": root["identity_sha256"],
                     "ply": root["ply"], "side_to_move": root["sfen"].split()[1],
                     "black_material_cp": value, "material_bucket": material_bucket(value),
                     "pair_outcome": outcome,
                     "split_winner_colour": None if outcome != "split" else
                         "Black" if first["result"] == "candidate_win" else "White"})
    summary = summarize(rows)
    expected_pairs = decision["pair_observations"]
    if (summary["outcomes"]["candidate_pair_win"] != expected_pairs["wins"] or
            summary["outcomes"]["candidate_pair_loss"] != expected_pairs["losses"] or
            summary["outcomes"]["split"] + summary["outcomes"]["double_draw"] != expected_pairs["draws"]):
        raise ValueError("finalized pair counts differ")
    groups = defaultdict(list)
    for row in rows:
        groups[row["material_bucket"]].append(row)
    return {"status": "POSTHOC_DIAGNOSTIC_ONLY", "trial": str(directory), "candidate": decision["candidate"],
            "old_verdict": decision["status"], "old_verdict_changed": False, "strength_claim": False,
            "inputs": {str(p): sha(p) for p in (plan_path, root_path, decision_path, result_path, events_path)},
            "material_definition": "static board+hand Black-minus-White; current documented PIECE_VALUE, not NNUE/search score",
            "summary": summary, "by_material": {key: summarize(groups[key]) for key in ("0", "1..300", "301..1000", ">1000")},
            "by_ply": {key: summarize([row for row in rows if low <= row["ply"] <= high])
                       for key, low, high in (("24..35", 24, 35), ("36..48", 36, 48))}, "rows": rows}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--trial-dir", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    result = analyze(args.trial_dir)
    with args.output.open("x") as stream:
        json.dump(result, stream, indent=2)
        stream.write("\n")
    print(json.dumps({key: result[key] for key in ("candidate", "summary", "by_material", "by_ply")}))


if __name__ == "__main__":
    main()
