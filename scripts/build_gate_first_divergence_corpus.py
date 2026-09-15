#!/usr/bin/env python3
"""Extract same-position candidate/baseline decision points from gate pairs.

Only the prefix before the first differing move is shared by both games.  A
later equal ply is not treated as a comparable position after the game trees
have diverged.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
from collections import defaultdict
from pathlib import Path

from diagnostic_contract import stable_entry_id
from summarize_gate_pair_outcomes import kifu_contract


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def moves_from_kifu(path: Path) -> list[str]:
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.startswith("position "):
            return line.split(" moves ", 1)[1].split() if " moves " in line else []
    raise ValueError(f"{path}: missing position")


def initial_sfen(contract: dict[str, str]) -> str:
    position = contract["initial_position"]
    prefix = "position sfen "
    if not position.startswith(prefix):
        raise ValueError("only SFEN gate records are supported")
    return position[len(prefix):]


def materialize(binary: Path, initial: str, history: list[str]) -> str:
    completed = subprocess.run(
        [str(binary), "--sfen", initial, "--moves", " ".join(history)],
        check=True, capture_output=True, text=True,
    )
    fields = dict(field.split("=", 1) for field in completed.stdout.strip().split("\t") if "=" in field)
    sfen = fields.get("sfen")
    if not sfen:
        raise ValueError("history materializer did not return SFEN")
    return sfen


def first_divergence(left: list[str], right: list[str]) -> int | None:
    for index, (a, b) in enumerate(zip(left, right, strict=False)):
        if a != b:
            return index
    return None


def entry_for_pair(pair: dict, replay_binary: Path) -> dict | None:
    kifu = pair.get("kifu")
    if not isinstance(kifu, list) or len(kifu) != 2:
        raise ValueError("pair requires exactly two kifu paths")
    paths = [Path(value) for value in kifu]
    contracts = [kifu_contract(path) for path in paths]
    if {contract["candidate_color"] for contract in contracts} != {"black", "white"}:
        raise ValueError("pair lacks color reversal")
    if len({contract["initial_position"] for contract in contracts}) != 1:
        raise ValueError("pair lacks shared initial position")
    moves = [moves_from_kifu(path) for path in paths]
    ply = first_divergence(moves[0], moves[1])
    if ply is None:
        return None
    initial = initial_sfen(contracts[0])
    history = moves[0][:ply]
    sfen = materialize(replay_binary, initial, history)
    side = sfen.split()[1] if len(sfen.split()) >= 2 else None
    side_name = {"b": "black", "w": "white"}.get(side)
    if side_name is None:
        raise ValueError("materialized SFEN lacks side to move")
    candidate_index = next((index for index, contract in enumerate(contracts)
                            if contract["candidate_color"] == side_name), None)
    if candidate_index is None:
        raise ValueError("cannot identify candidate actor at divergence")
    baseline_index = 1 - candidate_index
    source = {
        "run_id": Path(paths[0]).parents[1].name,
        "pair_id": pair["pair_id"],
        "game_id": f"{pair['pair_id']}:first-divergence",
        "ply": ply,
        "pair_class": pair.get("pair_class"),
        "candidate_color": side_name,
        "candidate_kifu": str(paths[candidate_index]),
        "baseline_kifu": str(paths[baseline_index]),
        "candidate_move_usi": moves[candidate_index][ply],
        "baseline_move_usi": moves[baseline_index][ply],
    }
    entry = {
        "source": source,
        "position": {
            "sfen": sfen,
            "initial_sfen": initial,
            "history_before_usi": history,
            "history_before": [],
            "actual_move_usi": moves[candidate_index][ply],
            "baseline_move_usi": moves[baseline_index][ply],
            "observed_score_cp": None,
            "reanalysis_score_cp": None,
        },
        "selection_reason": "first_same_position_candidate_baseline_divergence",
        "label_policy": "observation_only_no_correct_move_label",
    }
    entry["id"] = stable_entry_id(entry)
    return entry


def build(pair_document: dict, pair_outcomes_path: Path, replay_binary: Path, limit_per_class: int) -> dict:
    pairs = pair_document.get("pairs")
    if not isinstance(pairs, list):
        raise ValueError("pair outcome document lacks pairs")
    grouped: dict[str, list[dict]] = defaultdict(list)
    skipped = []
    for pair in pairs:
        try:
            entry = entry_for_pair(pair, replay_binary)
        except (OSError, ValueError, subprocess.CalledProcessError) as error:
            skipped.append({"pair_id": pair.get("pair_id"), "reason": str(error)})
            continue
        if entry is None:
            skipped.append({"pair_id": pair.get("pair_id"), "reason": "no_first_divergence"})
        else:
            grouped[str(entry["source"].get("pair_class"))].append(entry)
    selected = []
    for pair_class in ("candidate_sweep_loss", "candidate_sweep_win", "split", "draw_involved"):
        selected.extend(sorted(grouped[pair_class], key=lambda entry: entry["id"])[:limit_per_class])
    return {
        "schema": "sekirei.gate-first-divergence-corpus.v1",
        "diagnostic_only": True,
        "strength_claim": "not_permitted",
        "selection_contract": {
            "point": "first differing move after a shared replayed prefix",
            "max_entries_per_pair_class": limit_per_class,
            "selection": "stable_id_order",
        },
        "inputs": {
            "pair_outcomes": {"path": str(pair_outcomes_path), "sha256": sha256(pair_outcomes_path)},
            "execution_manifest": pair_document.get("execution_manifest"),
            "replay_binary": {"path": str(replay_binary), "sha256": sha256(replay_binary)},
        },
        "entries": selected,
        "available_by_pair_class": {kind: len(entries) for kind, entries in sorted(grouped.items())},
        "skipped": skipped,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pair-outcomes", type=Path, required=True)
    parser.add_argument("--replay-binary", type=Path, required=True)
    parser.add_argument("--limit-per-pair-class", type=int, default=4)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.limit_per_pair_class <= 0:
        parser.error("--limit-per-pair-class must be positive")
    try:
        document = build(json.loads(args.pair_outcomes.read_text(encoding="utf-8")),
                         args.pair_outcomes, args.replay_binary, args.limit_per_pair_class)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {len(document['entries'])} first-divergence positions")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
