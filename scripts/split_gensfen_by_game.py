#!/usr/bin/env python3
"""Create a leakage-resistant train/validation split for ``gensfen`` rows.

New ``gensfen`` output carries a sixth ``source_game_id`` column.  This tool
keeps a whole game in one arm.  Games sharing an exact SFEN are joined into
one component as an additional guard against cross-arm position leakage.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, Sequence


@dataclass(frozen=True)
class Record:
    raw: str
    sfen: str
    result: int
    ply: int
    game_id: str
    black_result: int


class UnionFind:
    def __init__(self, values: Iterable[str]) -> None:
        self.parent = {value: value for value in values}

    def find(self, value: str) -> str:
        parent = self.parent[value]
        if parent != value:
            self.parent[value] = self.find(parent)
        return self.parent[value]

    def union(self, left: str, right: str) -> None:
        left_root = self.find(left)
        right_root = self.find(right)
        if left_root == right_root:
            return
        if right_root < left_root:
            left_root, right_root = right_root, left_root
        self.parent[right_root] = left_root


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def parse_record(raw: str, source: Path, line_number: int) -> Record:
    columns = raw.rstrip("\n").split("\t")
    if len(columns) < 6:
        raise ValueError(
            f"{source}:{line_number}: expected the six-column gensfen format; "
            "regenerate legacy rows so train and validation can be split by game"
        )
    sfen, score_raw, result_raw, ply_raw, bestmove, game_id = columns[:6]
    sfen_fields = sfen.split()
    if len(sfen_fields) < 4 or sfen_fields[1] not in {"b", "w"}:
        raise ValueError(f"{source}:{line_number}: invalid SFEN side-to-move field")
    try:
        int(score_raw)
        result = int(result_raw)
        ply = int(ply_raw)
    except ValueError as error:
        raise ValueError(f"{source}:{line_number}: invalid numeric field") from error
    if result not in {-1, 0, 1}:
        raise ValueError(f"{source}:{line_number}: result must be -1, 0, or 1")
    if ply < 0:
        raise ValueError(f"{source}:{line_number}: ply must be non-negative")
    if not bestmove or not game_id or any(char.isspace() for char in game_id):
        raise ValueError(f"{source}:{line_number}: invalid bestmove or source_game_id")
    black_result = result if sfen_fields[1] == "b" else -result
    return Record(raw.rstrip("\n"), sfen, result, ply, game_id, black_result)


def load_records(paths: Sequence[Path]) -> tuple[list[Record], list[dict[str, object]]]:
    records: list[Record] = []
    inputs: list[dict[str, object]] = []
    for path in paths:
        before = len(records)
        with path.open(encoding="utf-8") as handle:
            for line_number, raw in enumerate(handle, 1):
                if raw.strip():
                    records.append(parse_record(raw, path, line_number))
        inputs.append(
            {
                "path": str(path),
                "sha256": sha256(path),
                "positions": len(records) - before,
            }
        )
    if not records:
        raise ValueError("no gensfen records found")
    return records, inputs


def validate_game_results(records: Sequence[Record]) -> dict[str, int]:
    results: dict[str, int] = {}
    for record in records:
        previous = results.setdefault(record.game_id, record.black_result)
        if previous != record.black_result:
            raise ValueError(
                f"source_game_id {record.game_id!r} has inconsistent game results: "
                f"{previous} and {record.black_result}"
            )
    return results


def connected_games(records: Sequence[Record]) -> list[set[str]]:
    game_ids = {record.game_id for record in records}
    groups = UnionFind(game_ids)
    sfen_owner: dict[str, str] = {}
    for record in records:
        owner = sfen_owner.setdefault(record.sfen, record.game_id)
        groups.union(owner, record.game_id)
    components: dict[str, set[str]] = {}
    for game_id in game_ids:
        components.setdefault(groups.find(game_id), set()).add(game_id)
    return list(components.values())


def component_key(component: set[str], seed: int) -> bytes:
    identity = "\n".join(sorted(component))
    return hashlib.sha256(f"{seed}\0{identity}".encode()).digest()


def choose_validation_games(
    records: Sequence[Record], components: Sequence[set[str]], ratio: float, seed: int
) -> set[str]:
    if not 0.0 < ratio < 1.0:
        raise ValueError("validation ratio must be between zero and one")
    if len(components) < 2:
        raise ValueError("at least two independent game components are required")
    sizes: dict[str, int] = {}
    for record in records:
        sizes[record.game_id] = sizes.get(record.game_id, 0) + 1
    ordered = sorted(components, key=lambda item: component_key(item, seed))
    cumulative = 0
    candidates: list[tuple[int, int]] = []
    for index, component in enumerate(ordered[:-1], 1):
        cumulative += sum(sizes[game_id] for game_id in component)
        candidates.append((abs(cumulative - round(len(records) * ratio)), index))
    _, boundary = min(candidates)
    return set().union(*ordered[:boundary])


def sample_game(indices: Sequence[int], limit: int) -> list[int]:
    if limit <= 0 or len(indices) <= limit:
        return list(indices)
    if limit == 1:
        return [indices[len(indices) // 2]]
    selected = {
        round(index * (len(indices) - 1) / (limit - 1)) for index in range(limit)
    }
    return [item for index, item in enumerate(indices) if index in selected]


def select_records(records: Sequence[Record], game_ids: set[str], limit: int) -> list[Record]:
    by_game: dict[str, list[int]] = {}
    for index, record in enumerate(records):
        if record.game_id in game_ids:
            by_game.setdefault(record.game_id, []).append(index)
    selected: set[int] = set()
    for game_indices in by_game.values():
        selected.update(sample_game(game_indices, limit))
    return [record for index, record in enumerate(records) if index in selected]


def phase_counts(records: Sequence[Record]) -> dict[str, int]:
    return {
        "opening_lt40": sum(record.ply < 40 for record in records),
        "middlegame_40_79": sum(40 <= record.ply < 80 for record in records),
        "endgame_ge80": sum(record.ply >= 80 for record in records),
    }


def result_counts(game_results: dict[str, int], game_ids: set[str]) -> dict[str, int]:
    return {
        "black_win": sum(game_results[game_id] == 1 for game_id in game_ids),
        "draw": sum(game_results[game_id] == 0 for game_id in game_ids),
        "white_win": sum(game_results[game_id] == -1 for game_id in game_ids),
    }


def atomic_write_lines(path: Path, records: Sequence[Record]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_suffix(path.suffix + ".tmp")
    with temporary.open("w", encoding="utf-8", newline="\n") as handle:
        for record in records:
            handle.write(record.raw + "\n")
    os.replace(temporary, path)


def game_ids_hash(game_ids: set[str]) -> str:
    payload = "".join(f"{game_id}\n" for game_id in sorted(game_ids)).encode()
    return hashlib.sha256(payload).hexdigest()


def split_files(
    inputs: Sequence[Path],
    train_out: Path,
    validation_out: Path,
    manifest_out: Path,
    validation_ratio: float,
    seed: int,
    max_positions_per_game: int,
) -> dict[str, object]:
    outputs = {train_out.resolve(), validation_out.resolve(), manifest_out.resolve()}
    if len(outputs) != 3 or any(path.resolve() in outputs for path in inputs):
        raise ValueError("inputs and outputs must all be distinct")
    records, input_manifest = load_records(inputs)
    game_results = validate_game_results(records)
    components = connected_games(records)
    validation_games = choose_validation_games(records, components, validation_ratio, seed)
    train_games = set(game_results) - validation_games
    train_records = select_records(records, train_games, max_positions_per_game)
    validation_records = select_records(records, validation_games, max_positions_per_game)
    overlap = {record.sfen for record in train_records} & {
        record.sfen for record in validation_records
    }
    if overlap:
        raise AssertionError("connected-component split left cross-arm SFEN overlap")

    atomic_write_lines(train_out, train_records)
    atomic_write_lines(validation_out, validation_records)
    manifest: dict[str, object] = {
        "schema": "sekirei.gensfen-game-split.v1",
        "inputs": input_manifest,
        "contract": {
            "unit": "source_game_id",
            "shared_sfen_games_are_joined": True,
            "seed": seed,
            "requested_validation_ratio": validation_ratio,
            "max_positions_per_game": max_positions_per_game or None,
        },
        "source": {
            "positions": len(records),
            "games": len(game_results),
            "components": len(components),
        },
        "train": {
            "path": str(train_out),
            "sha256": sha256(train_out),
            "positions": len(train_records),
            "games": len(train_games),
            "game_ids_sha256": game_ids_hash(train_games),
            "phase_counts": phase_counts(train_records),
            "game_results": result_counts(game_results, train_games),
        },
        "validation": {
            "path": str(validation_out),
            "sha256": sha256(validation_out),
            "positions": len(validation_records),
            "games": len(validation_games),
            "game_ids_sha256": game_ids_hash(validation_games),
            "phase_counts": phase_counts(validation_records),
            "game_results": result_counts(game_results, validation_games),
        },
        "observed_validation_ratio": len(validation_records)
        / (len(train_records) + len(validation_records)),
        "cross_arm_sfen_overlap": 0,
    }
    manifest_out.parent.mkdir(parents=True, exist_ok=True)
    temporary = manifest_out.with_suffix(manifest_out.suffix + ".tmp")
    temporary.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    os.replace(temporary, manifest_out)
    return manifest


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("inputs", nargs="+", type=Path)
    parser.add_argument("--train-out", required=True, type=Path)
    parser.add_argument("--validation-out", required=True, type=Path)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--validation-ratio", type=float, default=0.1)
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument(
        "--max-positions-per-game",
        type=int,
        default=0,
        help="deterministically thin long games; zero keeps every position",
    )
    args = parser.parse_args()
    if args.max_positions_per_game < 0:
        parser.error("--max-positions-per-game must be non-negative")
    try:
        report = split_files(
            args.inputs,
            args.train_out,
            args.validation_out,
            args.manifest,
            args.validation_ratio,
            args.seed,
            args.max_positions_per_game,
        )
    except (OSError, ValueError, json.JSONDecodeError) as error:
        parser.error(str(error))
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
