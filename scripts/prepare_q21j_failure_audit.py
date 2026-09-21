#!/usr/bin/env python3
"""Freeze one history-aware candidate decision from every Q21i match game.

For each game, select the candidate decision immediately before its first
300cp-or-larger evaluation collapse.  If no such collapse exists, select the
decision before the largest observed collapse.  Selection uses only the
candidate's saved online scores and the game result; it does not inspect any
new M/S/T re-search result.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path


SCHEMA = "sekirei.q21j-failure-audit-corpus.v1"
KIFU_HEADER = re.compile(r"^# Engine1: .+ \((Black|White)\)$")


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read_jsonl(path: Path) -> list[dict]:
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]


def parse_kifu(path: Path) -> tuple[str, list[str], str]:
    lines = path.read_text(encoding="utf-8").splitlines()
    header = KIFU_HEADER.match(lines[0]) if lines else None
    if header is None:
        raise ValueError(f"{path}: missing Engine1 color header")
    position = next((line for line in lines if line.startswith("position sfen ")), None)
    if position is None:
        raise ValueError(f"{path}: missing position line")
    payload = position.removeprefix("position sfen ")
    initial, separator, moves = payload.partition(" moves ")
    if not separator or not initial:
        raise ValueError(f"{path}: position line lacks moves")
    return initial, moves.split(), header.group(1)


def score_value(search: dict) -> tuple[str, int]:
    mate = search.get("score_mate")
    if mate is not None:
        value = int(mate)
        return "mate", 100_000 if value > 0 else -100_000
    cp = search.get("score_cp")
    if not isinstance(cp, int):
        raise ValueError("transcript search lacks cp or mate score")
    return "cp", cp


def engine1_pid(game1_rows: list[dict], initial_sfen: str) -> int:
    fields = initial_sfen.split()
    if len(fields) < 2 or fields[1] not in {"b", "w"}:
        raise ValueError("initial SFEN lacks side to move")
    black_seq_parity = 0 if fields[1] == "b" else 1
    row = next((item for item in game1_rows if item["seq"] % 2 == black_seq_parity), None)
    if row is None or not isinstance(row.get("pid"), int):
        raise ValueError("cannot identify Engine1 pid from game 1")
    return row["pid"]


def choose(candidate_rows: list[dict]) -> tuple[dict, dict | None, str, int | None]:
    if not candidate_rows:
        raise ValueError("game has no candidate decisions")
    if len(candidate_rows) == 1:
        return candidate_rows[0], None, "only_candidate_turn", None
    transitions = []
    for previous, current in zip(candidate_rows, candidate_rows[1:]):
        _, previous_score = score_value(previous["search"])
        _, current_score = score_value(current["search"])
        transitions.append((current_score - previous_score, previous, current))
    first_large = next((item for item in transitions if item[0] <= -300), None)
    if first_large is not None:
        delta, previous, current = first_large
        return previous, current, "first_candidate_score_drop_ge_300cp", delta
    delta, previous, current = min(transitions, key=lambda item: (item[0], item[1]["seq"]))
    return previous, current, "largest_candidate_score_drop_control", delta


def prepare(transcript_path: Path, result_path: Path, kifu_dir: Path) -> dict:
    transcript = read_jsonl(transcript_path)
    results = read_jsonl(result_path)
    games = sorted({row.get("game_num") for row in transcript})
    if games != list(range(1, len(games) + 1)) or len(results) != len(games):
        raise ValueError("transcript/result games are incomplete or non-contiguous")

    replay = {}
    for game in games:
        path = kifu_dir / f"game{game:04d}.txt"
        initial, moves, engine1_color = parse_kifu(path)
        expected_color = "Black" if game % 2 else "White"
        if engine1_color != expected_color:
            raise ValueError(f"game {game}: Engine1 color reversal mismatch")
        rows = sorted((row for row in transcript if row.get("game_num") == game), key=lambda row: row["seq"])
        if [row["seq"] for row in rows] != list(range(len(rows))):
            raise ValueError(f"game {game}: transcript sequence is incomplete")
        if len(rows) < len(moves) or any(row.get("raw_bestmove") != move for row, move in zip(rows, moves)):
            raise ValueError(f"game {game}: kifu/transcript move mismatch")
        terminal_rows = rows[len(moves):]
        if len(terminal_rows) > 1 or any(row.get("raw_bestmove") not in {"resign", "win"} for row in terminal_rows):
            raise ValueError(f"game {game}: unexpected transcript rows after the recorded moves")
        replay[game] = {"path": path, "initial_sfen": initial, "moves": moves, "rows": rows}

    candidate_pid = engine1_pid(replay[1]["rows"], replay[1]["initial_sfen"])
    positions = []
    for game, result in zip(games, results):
        item = replay[game]
        candidate_rows = [
            row for row in item["rows"]
            if row.get("pid") == candidate_pid and row.get("raw_bestmove") not in {"resign", "win"}
        ]
        selected, next_row, reason, delta = choose(candidate_rows)
        seq = selected["seq"]
        before_kind, before_score = score_value(selected["search"])
        after_kind, after_score = score_value(next_row["search"]) if next_row else (None, None)
        positions.append({
            "id": f"q21j-game-{game:02d}",
            "game_num": game,
            "game_result": result.get("result"),
            "selection": {
                "rule": reason,
                "selected_seq": seq,
                "next_candidate_seq": next_row.get("seq") if next_row else None,
                "observed_score_kind": before_kind,
                "observed_score": before_score,
                "next_observed_score_kind": after_kind,
                "next_observed_score": after_score,
                "observed_delta": delta,
            },
            "position": {
                "initial_sfen": item["initial_sfen"],
                "history_before_usi": item["moves"][:seq],
                "sfen": selected["sfen_before"],
                "actual_move_usi": selected["raw_bestmove"],
            },
            "source": {"kifu": str(item["path"]), "kifu_sha256": sha256(item["path"])},
        })
    if len(positions) != 32 or len({row["id"] for row in positions}) != 32:
        raise ValueError("Q21j requires exactly one decision from each of 32 games")
    return {
        "schema": SCHEMA,
        "diagnostic_only": True,
        "strength_claim": False,
        "selection_blinding": "saved candidate online scores and game result only; no new re-search output",
        "candidate_pid": candidate_pid,
        "inputs": {
            "transcript": {"path": str(transcript_path), "sha256": sha256(transcript_path)},
            "result_records": {"path": str(result_path), "sha256": sha256(result_path)},
            "kifu_dir": str(kifu_dir),
        },
        "counts": {
            "games": len(games),
            "candidate_wins": sum(row["game_result"] == "candidate_win" for row in positions),
            "candidate_losses": sum(row["game_result"] == "baseline_win" for row in positions),
            "first_large_drop": sum(row["selection"]["rule"] == "first_candidate_score_drop_ge_300cp" for row in positions),
        },
        "positions": positions,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--transcript", type=Path, required=True)
    parser.add_argument("--result-records", type=Path, required=True)
    parser.add_argument("--kifu-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        document = prepare(args.transcript, args.result_records, args.kifu_dir)
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(document["counts"], sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
