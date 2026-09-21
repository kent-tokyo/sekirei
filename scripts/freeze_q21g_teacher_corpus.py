#!/usr/bin/env python3
"""Freeze the history-aware Q21g teacher-judgment audit corpus.

The selection is deterministic and deliberately independent of any evaluator
score.  It aims for 96 normal and 32 tactical positions while covering every
phase x material stratum.  Empty/terminal-only replays remain documented as
excluded sources rather than being silently replaced.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any

from compare_nnue_root_profiles import attributes
from run_fixed_selfplay_diagnostic import csa_move_to_usi


PHASES = ("opening", "middlegame", "endgame")
MATERIAL_BANDS = ("stm_behind", "balanced", "stm_ahead")
STRATA = tuple((phase, material) for phase in PHASES for material in MATERIAL_BANDS)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def stable_rank(value: str) -> str:
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


def tactical_flags(
    position: dict[str, Any], next_position: dict[str, Any], actual_move_usi: str
) -> dict[str, bool]:
    in_check = bool(position.get("side_to_move_in_check"))
    gives_check = bool(next_position.get("side_to_move_in_check"))
    return {
        "in_check": in_check,
        "gives_check": gives_check,
        "king_threat": in_check or gives_check,
        "capture": position.get("captured_piece") is not None,
        "promotion": actual_move_usi.endswith("+"),
        "drop": bool(position.get("actual_move_is_drop")),
    }


def replay_candidates(replays: Path) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    candidates: list[dict[str, Any]] = []
    excluded: list[dict[str, Any]] = []
    for path in sorted(replays.glob("game*.json")):
        document = json.loads(path.read_text(encoding="utf-8"))
        positions = document.get("positions")
        if not isinstance(positions, list) or len(positions) < 2:
            excluded.append(
                {
                    "path": str(path),
                    "sha256": sha256(path),
                    "reason": "fewer_than_two_replay_positions",
                    "positions": len(positions) if isinstance(positions, list) else None,
                }
            )
            continue
        source_id = str(document.get("game_id") or path.stem)
        for index, position in enumerate(positions[:-1]):
            next_position = positions[index + 1]
            actual_move_usi = csa_move_to_usi(
                position["pre_move_sfen"], position["actual_move_csa"]
            )
            next_history = next_position.get("history_before_usi")
            if not isinstance(next_history, list) or not next_history:
                raise ValueError(f"{path}: next position does not preserve history")
            if next_history[-1] != actual_move_usi:
                raise ValueError(
                    f"{path}: CSA/USI history mismatch at ply {position.get('ply')}: "
                    f"{actual_move_usi} != {next_history[-1]}"
                )
            history = position.get("history_before_usi")
            if not isinstance(history, list):
                raise ValueError(f"{path}: invalid history_before_usi")
            position_attributes = attributes(position["pre_move_sfen"])
            flags = tactical_flags(position, next_position, actual_move_usi)
            tactical = any(flags.values())
            candidate_id = f"{path.stem}-ply{int(position['ply']):03d}"
            candidates.append(
                {
                    "id": candidate_id,
                    "selection_class": "tactical" if tactical else "normal",
                    "source": {
                        "game_id": source_id,
                        "replay_path": str(path),
                        "replay_sha256": sha256(path),
                        "source_csa_path": document.get("source_csa_path"),
                        "result": document.get("result"),
                        "terminal_complete": document.get("terminal_complete"),
                    },
                    "position": {
                        "initial_sfen": document["initial_sfen"],
                        "history_before_usi": history,
                        "sfen": position["pre_move_sfen"],
                        "ply": int(position["ply"]),
                        "side_to_move": position["side_to_move"],
                        "actual_move_csa": position["actual_move_csa"],
                        "actual_move_usi": actual_move_usi,
                        "observed_score_cp": position.get("observed_score_cp"),
                        "prior_reanalysis_score_cp": position.get("reanalysis_score_cp"),
                    },
                    "attributes": position_attributes,
                    "tactical_flags": flags,
                    "selection_rank": stable_rank(candidate_id),
                }
            )
    return candidates, excluded


def _pick_class(
    candidates: list[dict[str, Any]],
    selection_class: str,
    target: int,
    selected_ids: set[str],
    source_counts: Counter[str],
) -> list[dict[str, Any]]:
    pools: dict[tuple[str, str], list[dict[str, Any]]] = defaultdict(list)
    for row in candidates:
        if row["selection_class"] != selection_class:
            continue
        key = (row["attributes"]["phase"], row["attributes"]["material_band"])
        pools[key].append(row)
    for pool in pools.values():
        pool.sort(key=lambda row: (row["selection_rank"], row["id"]))

    picked: list[dict[str, Any]] = []
    stratum_counts: Counter[tuple[str, str]] = Counter()
    while len(picked) < target:
        available: list[tuple[int, int, str, dict[str, Any]]] = []
        for stratum in STRATA:
            choices = [
                row
                for row in pools.get(stratum, [])
                if row["id"] not in selected_ids and source_counts[row["source"]["game_id"]] < 2
            ]
            if not choices:
                continue
            minimum_source_use = min(source_counts[row["source"]["game_id"]] for row in choices)
            row = next(
                row
                for row in choices
                if source_counts[row["source"]["game_id"]] == minimum_source_use
            )
            available.append(
                (
                    minimum_source_use,
                    stratum_counts[stratum],
                    row["selection_rank"],
                    row,
                )
            )
        if not available:
            raise ValueError(f"cannot fill {selection_class} target {target}; got {len(picked)}")
        _, _, _, row = min(available, key=lambda item: item[:3])
        picked.append(row)
        selected_ids.add(row["id"])
        source_counts[row["source"]["game_id"]] += 1
        stratum_counts[(row["attributes"]["phase"], row["attributes"]["material_band"])] += 1
    return picked


def freeze(candidates: list[dict[str, Any]], normal: int, tactical: int) -> list[dict[str, Any]]:
    selected_ids: set[str] = set()
    source_counts: Counter[str] = Counter()
    selected = _pick_class(candidates, "normal", normal, selected_ids, source_counts)
    selected += _pick_class(candidates, "tactical", tactical, selected_ids, source_counts)
    return sorted(selected, key=lambda row: (row["selection_class"], row["selection_rank"]))


def counts(rows: list[dict[str, Any]]) -> dict[str, Any]:
    source_counts = Counter(row["source"]["game_id"] for row in rows)
    strata = Counter(
        (row["attributes"]["phase"], row["attributes"]["material_band"])
        for row in rows
    )
    flags = Counter(
        flag
        for row in rows
        for flag, enabled in row["tactical_flags"].items()
        if enabled
    )
    return {
        "positions": len(rows),
        "normal": sum(row["selection_class"] == "normal" for row in rows),
        "tactical": sum(row["selection_class"] == "tactical" for row in rows),
        "unique_sources": len(source_counts),
        "maximum_positions_per_source": max(source_counts.values(), default=0),
        "phase_material": {
            f"{phase}:{material}": strata[(phase, material)]
            for phase, material in STRATA
        },
        "tactical_flags": dict(sorted(flags.items())),
        "missing_phase_material": [
            f"{phase}:{material}"
            for phase, material in STRATA
            if strata[(phase, material)] == 0
        ],
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--replays", type=Path, required=True)
    parser.add_argument("--dataset-manifest", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--normal", type=int, default=96)
    parser.add_argument("--tactical", type=int, default=32)
    args = parser.parse_args()
    if args.normal < 0 or args.tactical < 0 or args.normal + args.tactical > 128:
        parser.error("normal/tactical counts must be non-negative and total at most 128")
    candidates, excluded = replay_candidates(args.replays)
    selected = freeze(candidates, args.normal, args.tactical)
    summary = counts(selected)
    document = {
        "schema": "sekirei.q21g-teacher-corpus.v1",
        "status": "frozen-development-diagnostic",
        "diagnostic_only": True,
        "strength_claim": False,
        "selection_contract": {
            "score_blind": True,
            "target_normal": args.normal,
            "target_tactical": args.tactical,
            "phase_rule": "SFEN move number: opening<=20, middlegame<=60, endgame>60",
            "material_rule": "engine material from side-to-move; balanced iff abs(score)<=200cp",
            "tactical_rule": "in-check, gives-check, capture, promotion, or drop",
            "ordering": "prefer unused source, then least-filled phase/material stratum; SHA-256 tie-break",
            "source_cap": 2,
            "last_replay_position_excluded": "actual USI move cannot be independently cross-checked against next history",
        },
        "inputs": {
            "replays": str(args.replays),
            "dataset_manifest": str(args.dataset_manifest),
            "dataset_manifest_sha256": sha256(args.dataset_manifest),
        },
        "excluded_sources": excluded,
        "summary": summary,
        "entries": selected,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(summary, ensure_ascii=False, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
