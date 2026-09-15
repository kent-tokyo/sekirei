"""Shared identity and score-kind contract for local diagnostic artifacts.

The game-relative row number is deliberately not an identity: reports can be
sorted or filtered without changing which position a later deep search means.
"""
from __future__ import annotations

import hashlib
import json


MATE_THRESHOLD = 899_000


def stable_entry_id(entry: dict) -> str:
    source = entry.get("source")
    position = entry.get("position")
    if not isinstance(source, dict) or not isinstance(position, dict):
        raise ValueError("diagnostic entry needs source and position")
    game_id, ply, sfen = source.get("game_id"), source.get("ply"), position.get("sfen")
    if not isinstance(game_id, str) or not game_id or not isinstance(ply, int) or ply < 0:
        raise ValueError("diagnostic entry needs source game_id and non-negative ply")
    if not isinstance(sfen, str) or not sfen:
        raise ValueError("diagnostic entry needs position SFEN")
    identity = {
        "run_id": source.get("run_id"),
        "pair_id": source.get("pair_id"),
        "game_id": game_id,
        "ply": ply,
        "sfen": sfen,
        "initial_sfen": position.get("initial_sfen"),
        "history_before_usi": position.get("history_before_usi"),
    }
    encoded = json.dumps(identity, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode()
    return f"diag-{hashlib.sha256(encoded).hexdigest()[:20]}"


def score_kind(score: object) -> str:
    if not isinstance(score, int):
        return "missing"
    return "mate" if abs(score) >= MATE_THRESHOLD else "cp"
