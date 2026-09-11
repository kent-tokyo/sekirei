#!/usr/bin/env python3
"""Align a CSA game with its analysis sidecar and flag evaluation swings."""
import argparse
import json
import re
import sys
from pathlib import Path

MOVE = re.compile(r"^[+-][0-9]{4}[A-Z]{2}$")
MATE_SCORE_CP = 800_000


def load(path):
    docs = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
    if not docs or docs[0].get("schema") != "sekirei.analysis-record.v1":
        raise ValueError("analysis sidecar has no v1 header")
    if docs[0].get("engine") != "sekirei" or not docs[0].get("engine_version"):
        raise ValueError("analysis sidecar has incomplete engine identity")
    if docs[0].get("score_perspective") != "side_to_move":
        raise ValueError("analysis sidecar has unsupported score perspective")
    return docs[0], [doc for doc in docs[1:] if doc.get("type") == "search"]


def csa_moves(path):
    return [line.split(",", 1)[0] for line in path.read_text(encoding="utf-8").splitlines() if MOVE.fullmatch(line.split(",", 1)[0])]


def csa_game_id(path):
    prefix = "$EVENT:"
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.startswith(prefix):
            return line[len(prefix):]
    return None


def csa_result(path):
    for line in reversed(path.read_text(encoding="utf-8").splitlines()):
        if line in {"#WIN", "#LOSE", "#DRAW", "#JISHOGI"}:
            return {"#WIN": "win", "#LOSE": "lose", "#DRAW": "draw", "#JISHOGI": "draw"}[line]
    return None


def csa_is_jishogi(path):
    return "#JISHOGI" in path.read_text(encoding="utf-8").splitlines()


def phase_for_ply(ply, moves):
    if not isinstance(ply, int) or moves <= 0:
        return "unknown"
    ratio = ply / moves
    if ratio < 0.25:
        return "opening"
    if ratio < 0.75:
        return "middlegame"
    return "endgame"


def depth_band(depth):
    if not isinstance(depth, int):
        return "unknown"
    if depth < 8:
        return "shallow"
    if depth <= 15:
        return "medium"
    return "deep"


def elapsed_band(elapsed_ms):
    if not isinstance(elapsed_ms, int):
        return "unknown"
    if elapsed_ms < 1_000:
        return "fast"
    if elapsed_ms < 10_000:
        return "normal"
    return "slow"


def analyze(csa_path, analysis_path, threshold=200):
    header, records = load(analysis_path)
    all_docs = [json.loads(line) for line in analysis_path.read_text(encoding="utf-8").splitlines() if line.strip()]
    recorded_end = next((doc.get("result") for doc in all_docs if doc.get("type") == "game_end"), None)
    moves = csa_moves(csa_path)
    aligned = []
    errors = []
    event = csa_game_id(csa_path)
    if event is not None and header.get("game_id") != event:
        errors.append("CSA $EVENT does not match analysis game_id")
    csa_outcome = csa_result(csa_path)
    end = "draw" if recorded_end == "aborted" and csa_is_jishogi(csa_path) else recorded_end
    if end is not None and csa_outcome is not None and end != csa_outcome:
        errors.append("CSA result does not match analysis game_end")
    previous_score = None
    previous_ply = None
    previous_terminal = False
    for record in records:
        ply = record.get("ply")
        actual = moves[ply] if isinstance(ply, int) and 0 <= ply < len(moves) else None
        score = record.get("score_cp")
        selected = record.get("bestmove_csa")
        prior_score = previous_score
        prior_ply = previous_ply
        terminal = False
        terminal_reason = None
        if isinstance(score, int) and abs(score) >= MATE_SCORE_CP:
            terminal = True
            terminal_reason = "mate_score"
        elif actual is None and selected is None and ply == len(moves):
            terminal = True
            terminal_reason = "game_end"
        delta = (
            score - previous_score
            if isinstance(score, int)
            and isinstance(prior_score, int)
            and not previous_terminal
            and not terminal
            else None
        )
        if isinstance(score, int):
            previous_score = score
            previous_terminal = terminal
        if actual is not None:
            expected_side = "black" if actual.startswith("+") else "white"
            if record.get("side_to_move") != expected_side:
                errors.append(f"analysis ply {ply}: side_to_move does not match CSA")
            if selected is not None and selected[:1] != actual[:1]:
                errors.append(f"analysis ply {ply}: bestmove color does not match CSA")
        aligned.append({
            "ply": ply,
            "previous_ply": prior_ply,
            "previous_score_cp": prior_score,
            "phase": phase_for_ply(ply, len(moves)),
            "depth_band": depth_band(record.get("depth")),
            "elapsed_band": elapsed_band(record.get("elapsed_ms")),
            "sfen": record.get("sfen"),
            "actual_move_csa": actual,
            "bestmove_csa": selected,
            "bestmove_matches_actual": actual == selected if actual is not None and selected is not None else None,
            "score_cp": score,
            "score_delta_cp": delta,
            "terminal": terminal,
            "terminal_reason": terminal_reason,
            "swing": abs(delta) >= threshold and not terminal if delta is not None else False,
            "depth": record.get("depth"),
            "nodes": record.get("nodes"),
            "elapsed_ms": record.get("elapsed_ms"),
        })
        if actual is None and not terminal:
            errors.append(f"analysis ply {ply!r} is not present in CSA move list")
        previous_ply = ply
    return {
        "schema": "sekirei.analysis-swing-report.v1",
        "analysis_schema": header["schema"],
        "csa": str(csa_path),
        "analysis": str(analysis_path),
        "game_id": header.get("game_id"),
        "result": end,
        "recorded_result": recorded_end,
        "csa_result": csa_outcome,
        "moves": len(moves),
        "searches": len(records),
        "threshold_cp": threshold,
        "bestmove_mismatches": sum(x["bestmove_matches_actual"] is False for x in aligned),
            "swings": sum(x["swing"] for x in aligned),
            "normal_swings": sum(x["swing"] for x in aligned),
            "negative_swings": sum(x["swing"] and x["score_delta_cp"] < 0 for x in aligned),
            "positive_swings": sum(x["swing"] and x["score_delta_cp"] > 0 for x in aligned),
            "terminal_records": sum(x["terminal"] for x in aligned),
        "alignment_errors": errors,
        "records": aligned,
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("csa", type=Path)
    parser.add_argument("analysis", type=Path)
    parser.add_argument("--threshold-cp", type=int, default=200)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args(argv)
    try:
        report = analyze(args.csa, args.analysis, args.threshold_cp)
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        print(f"analysis failed: {exc}", file=sys.stderr)
        return 2
    text = json.dumps(report, ensure_ascii=False, indent=2) + "\n"
    if args.output:
        args.output.write_text(text, encoding="utf-8")
    else:
        print(text, end="")
    return 0 if not report["alignment_errors"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
