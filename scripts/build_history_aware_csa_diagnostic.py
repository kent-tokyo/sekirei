#!/usr/bin/env python3
"""Build a bounded USI diagnostic corpus without discarding CSA history.

Input files are ``sekirei.csa-replay.v2`` exports.  Each selected entry keeps
the initial SFEN, the legal USI prefix, and the independently materialized
pre-move SFEN.  Consumers can therefore verify both replay correctness and
history-dependent search behavior such as fourfold repetition.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


SCHEMA = "sekirei.history-aware-csa-diagnostic.v1"
PROMOTED_CSA_KINDS = {"TO", "NY", "NK", "NG", "UM", "RY"}


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def select_positions(document: dict, source: Path, limit: int) -> tuple[list[dict], str | None]:
    if document.get("schema") != "sekirei.csa-replay.v2":
        return [], "unsupported_replay_schema"
    initial_sfen = document.get("initial_sfen")
    positions = document.get("positions")
    if not isinstance(initial_sfen, str) or not initial_sfen or not isinstance(positions, list):
        return [], "missing_initial_sfen_or_positions"

    candidates: list[tuple[dict, str]] = []
    if positions:
        candidates.append((positions[0], "opening_control"))
    # These positions occur *after* a drop or a promotion-shaped CSA move, so
    # their replay prefix exercises hand and promoted-piece do/undo state.
    for position in positions:
        history = position.get("history_before", [])
        if isinstance(history, list) and history:
            previous = history[-1]
            if isinstance(previous, str) and len(previous) == 7:
                if previous[1:3] == "00":
                    candidates.append((position, "after_drop"))
                if previous[5:7] in PROMOTED_CSA_KINDS:
                    candidates.append((position, "after_promoted_piece_move"))
    for position in positions:
        if position.get("captured_piece") is not None:
            candidates.append((position, "capture_position"))
    for position in positions:
        if position.get("side_to_move_in_check") is True:
            candidates.append((position, "check_evasion"))
    candidates.extend((position, "deterministic_fill") for position in positions)

    selected: list[dict] = []
    seen: set[int] = set()
    for position, category in candidates:
        ply = position.get("ply")
        history = position.get("history_before_usi")
        sfen = position.get("pre_move_sfen")
        if not isinstance(ply, int) or ply in seen or not isinstance(sfen, str):
            continue
        if not isinstance(history, list) or not all(isinstance(move, str) and move for move in history):
            return [], f"invalid_usi_history_at_ply_{ply}"
        seen.add(ply)
        selected.append({
            "id": f"{source.stem}-ply{ply:03d}",
            "category": category,
            "initial_sfen": initial_sfen,
            "history_before_usi": history,
            "sfen": sfen,
            "source": {
                "replay": str(source),
                "replay_sha256": sha256(source),
                "ply": ply,
            },
        })
        if len(selected) == limit:
            break
    return selected, None


def build(replay_dir: Path, limit_per_game: int, max_games: int | None = None) -> dict:
    positions: list[dict] = []
    invalid: list[dict] = []
    duplicate_history_excluded: list[dict] = []
    seen_history: set[tuple[str, tuple[str, ...]]] = set()
    sources = sorted(replay_dir.glob("*.json"))
    if max_games is not None:
        sources = sources[:max_games]
    for source in sources:
        try:
            selected, error = select_positions(json.loads(source.read_text(encoding="utf-8")), source, limit_per_game)
        except (OSError, json.JSONDecodeError) as exc:
            selected, error = [], str(exc)
        for position in selected:
            identity = (position["initial_sfen"], tuple(position["history_before_usi"]))
            if identity in seen_history:
                duplicate_history_excluded.append({
                    "id": position["id"],
                    "category": position["category"],
                    "reason": "duplicate_initial_sfen_and_usi_history",
                })
                continue
            seen_history.add(identity)
            positions.append(position)
        if error is not None:
            invalid.append({"replay": str(source), "error": error})
    # `positions` feeds the USI driver; `entries` is the equivalent compact
    # view for the direct-core diagnostic. Keeping both views in one artifact
    # prevents cold/warm diagnostics from silently replacing history with a
    # materialized current SFEN.
    entries = [
        {
            "source": {
                "game_id": position["id"].rsplit("-ply", 1)[0],
                "replay": position["source"]["replay"],
                "replay_sha256": position["source"]["replay_sha256"],
                "ply": position["source"]["ply"],
                "category": position["category"],
            },
            "position": {
                "sfen": position["sfen"],
                "initial_sfen": position["initial_sfen"],
                "history_before": [],
                "history_before_usi": position["history_before_usi"],
                "actual_move_observed": None,
            },
        }
        for position in positions
    ]
    return {
        "schema": SCHEMA,
        "diagnostic_only": True,
        "history_mode": "initial_sfen_plus_usi_prefix",
        "positions": positions,
        "entries": entries,
        "invalid_replays": invalid,
        "duplicate_history_excluded": duplicate_history_excluded,
        "strength_claim": "not_permitted",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--replay-dir", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--max-per-game", type=int, default=3)
    parser.add_argument("--max-games", type=int, help="optional deterministic cap on replay files")
    args = parser.parse_args()
    if args.max_per_game <= 0:
        parser.error("--max-per-game must be positive")
    if args.max_games is not None and args.max_games <= 0:
        parser.error("--max-games must be positive")
    document = build(args.replay_dir, args.max_per_game, args.max_games)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {len(document['positions'])} positions, {len(document['invalid_replays'])} invalid replays")
    return 0 if not document["invalid_replays"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
