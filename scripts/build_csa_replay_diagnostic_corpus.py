#!/usr/bin/env python3
"""Select bounded, replay-verified diagnostic positions from CSA-only exports.

The selection is evidence-first: an opponent capture immediately before our
turn, our king being in check, and an opening control position have fixed
priority. Historic engine scores are deliberately not inferred from CSA.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


SCHEMA = "sekirei.floodgate-csa-replay-corpus.v1"


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def select_game(document: dict, source_path: Path, limit: int) -> tuple[list[dict], dict | None]:
    if document.get("schema") not in {"sekirei.csa-replay.v1", "sekirei.csa-replay.v2"}:
        return [], {"path": str(source_path), "error": "unsupported_replay_schema"}
    player_color = document.get("player_color")
    if player_color not in {"black", "white"}:
        return [], {"path": str(source_path), "error": "player_color_missing"}
    positions = document.get("positions")
    if not isinstance(positions, list):
        return [], {"path": str(source_path), "error": "positions_missing"}

    ours = [position for position in positions if position.get("is_player_to_move")]
    seen: set[int] = set()
    selected: list[tuple[dict, str]] = []

    def add(position: dict, reason: str) -> None:
        ply = position.get("ply")
        if not isinstance(ply, int) or ply in seen or len(selected) >= limit:
            return
        if not isinstance(position.get("pre_move_sfen"), str) or not isinstance(position.get("actual_move_csa"), str):
            return
        seen.add(ply)
        selected.append((position, reason))

    # A capture is recorded on the preceding opponent move. The following
    # player-to-move position is the first chance to respond to the loss.
    by_ply = {position.get("ply"): position for position in positions}
    for position in ours:
        previous = by_ply.get(position.get("ply", 0) - 1)
        if previous and previous.get("captured_color") == player_color:
            add(position, "first_or_next_opponent_capture_of_player_piece")
    for position in ours:
        if position.get("side_to_move_in_check"):
            add(position, "player_to_move_in_check")
    if ours:
        add(ours[0], "opening_control")
    for position in ours:
        add(position, "deterministic_fill")

    game_id = document.get("game_id") or source_path.stem
    rows = []
    for position, reason in selected:
        rows.append({
            "source": {
                "game_id": game_id,
                "csa": document.get("source_csa_path"),
                "replay": str(source_path),
                "replay_sha256": sha256(source_path),
                "ply": position["ply"],
                "result": document.get("result"),
                "terminal_complete": document.get("terminal_complete"),
            },
            "position": {
                "sfen": position["pre_move_sfen"],
                "initial_sfen": document.get("initial_sfen"),
                "history_before": position.get("history_before", []),
                "history_before_usi": position.get("history_before_usi", []),
                "actual_move_observed": position["actual_move_csa"],
                "side_to_move": position.get("side_to_move"),
                "is_player_to_move": True,
                "side_to_move_in_check": position.get("side_to_move_in_check"),
                "previous_opponent_capture": by_ply.get(position["ply"] - 1, {}).get("captured_piece"),
                "observed_score_cp": None,
                "reanalysis_score_cp": None,
            },
            "selection_reason": reason,
            "label_policy": "observation_only_no_correct_move_label",
        })
    return rows, None


def build(replay_dir: Path, limit_per_game: int) -> dict:
    entries: list[dict] = []
    invalid: list[dict] = []
    for path in sorted(replay_dir.glob("*.json")):
        try:
            rows, error = select_game(json.loads(path.read_text(encoding="utf-8")), path, limit_per_game)
        except (OSError, json.JSONDecodeError) as exc:
            rows, error = [], {"path": str(path), "error": str(exc)}
        entries.extend(rows)
        if error:
            invalid.append(error)
    return {
        "schema": SCHEMA,
        "diagnostic_only": True,
        "selection_contract": {
            "max_positions_per_game": limit_per_game,
            "priority": ["opponent_capture", "player_in_check", "opening_control", "deterministic_fill"],
            "historic_scores": "missing_not_inferred",
        },
        "entries": entries,
        "invalid_games": invalid,
        "strength_claim": "not_permitted",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--replay-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--max-per-game", type=int, default=4)
    args = parser.parse_args()
    if args.max_per_game <= 0:
        parser.error("--max-per-game must be positive")
    document = build(args.replay_dir, args.max_per_game)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {len(document['entries'])} positions, {len(document['invalid_games'])} invalid games")
    return 0 if not document["invalid_games"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
