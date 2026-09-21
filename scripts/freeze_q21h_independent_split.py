#!/usr/bin/env python3
"""Audit local replays and freeze a leakage-resistant Q21h NNUE split.

The unit of independence is a connected group of games, not a position.  Games
with the same (or symmetric) opening, or with any exact/symmetric position in
common, remain in one split.  Exclusion inputs are inspected recursively and
their position identities are recorded in an explicit ledger.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any, Iterable


ROOT = Path(__file__).resolve().parents[1]
PROFILE_SPEC = importlib.util.spec_from_file_location(
    "nnue_root_profiles", ROOT / "scripts" / "compare_nnue_root_profiles.py"
)
assert PROFILE_SPEC and PROFILE_SPEC.loader
PROFILES = importlib.util.module_from_spec(PROFILE_SPEC)
PROFILE_SPEC.loader.exec_module(PROFILES)

PHASES = ("opening", "middlegame", "endgame")
MATERIAL_BANDS = ("stm_behind", "balanced", "stm_ahead")
STRATA = tuple(f"{phase}/{material}" for phase in PHASES for material in MATERIAL_BANDS)
SFEN_SUFFIXES = {".json", ".jsonl", ".sfen"}
HAND_ORDER = "RBGSNLP rbgsnlp".replace(" ", "")


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _normalize_hand(hand: str) -> str:
    if hand == "-":
        return hand
    counts: Counter[str] = Counter()
    index = 0
    while index < len(hand):
        count_start = index
        while index < len(hand) and hand[index].isdigit():
            index += 1
        count = int(hand[count_start:index]) if index > count_start else 1
        if index >= len(hand) or hand[index] not in HAND_ORDER:
            raise ValueError(f"invalid SFEN hand: {hand!r}")
        counts[hand[index]] += count
        index += 1
    return "".join((str(counts[piece]) if counts[piece] > 1 else "") + piece for piece in HAND_ORDER if counts[piece]) or "-"


def canonical_sfen(sfen: str) -> str:
    fields = sfen.split()
    if len(fields) < 3 or fields[0].count("/") != 8 or fields[1] not in {"b", "w"}:
        raise ValueError(f"invalid SFEN: {sfen!r}")
    return f"{fields[0]} {fields[1]} {_normalize_hand(fields[2])}"


def _board_tokens(rank: str) -> list[str]:
    result: list[str] = []
    index = 0
    while index < len(rank):
        token = rank[index]
        if token == "+":
            if index + 1 >= len(rank):
                raise ValueError(f"invalid promoted token in rank: {rank!r}")
            result.append(rank[index : index + 2])
            index += 2
        else:
            result.append(token)
            index += 1
    return result


def _swap_piece_case(token: str) -> str:
    if token.isdigit():
        return token
    if token.startswith("+"):
        return "+" + token[1].swapcase()
    return token.swapcase()


def _swap_hand_case(hand: str) -> str:
    swapped = hand if hand == "-" else "".join(char.swapcase() if char.isalpha() else char for char in hand)
    return _normalize_hand(swapped)


def transform_sfen(sfen: str, *, mirror: bool, rotate_swap: bool) -> str:
    board, side, hand = canonical_sfen(sfen).split()
    ranks = [_board_tokens(rank) for rank in board.split("/")]
    if mirror:
        ranks = [list(reversed(rank)) for rank in ranks]
    if rotate_swap:
        ranks = [[_swap_piece_case(token) for token in reversed(rank)] for rank in reversed(ranks)]
        side = "w" if side == "b" else "b"
        hand = _swap_hand_case(hand)
    return f"{'/'.join(''.join(rank) for rank in ranks)} {side} {hand}"


def symmetry_key(sfen: str) -> str:
    variants = {
        transform_sfen(sfen, mirror=mirror, rotate_swap=rotate_swap)
        for mirror in (False, True)
        for rotate_swap in (False, True)
    }
    return min(variants)


def stable_rank(value: str, seed: int) -> str:
    return hashlib.sha256(f"{seed}\0{value}".encode()).hexdigest()


def replay_phase(initial_sfen: str, row: dict[str, Any]) -> tuple[str, int]:
    initial_move = int(initial_sfen.split()[3])
    ply = int(row["ply"])
    absolute_move = initial_move + ply
    row_move = int(row["pre_move_sfen"].split()[3])
    if row_move != absolute_move:
        raise ValueError(
            f"replay/SFEN move mismatch: initial={initial_move}, ply={ply}, sfen={row_move}"
        )
    phase = "opening" if absolute_move <= 20 else "middlegame" if absolute_move <= 60 else "endgame"
    return phase, absolute_move


def mate_status(row: dict[str, Any]) -> str:
    for key in ("observed_score_kind", "reanalysis_score_kind", "score_kind"):
        value = row.get(key)
        if value == "mate":
            return "mate"
        if value == "cp":
            return "non_mate"
    # A numeric cp value does not prove that the search did not also have a
    # mate line; old logs did not preserve the score kind.
    return "unknown"


def position_row(game: dict[str, Any], replay: dict[str, Any], index: int) -> dict[str, Any]:
    position = replay["positions"][index]
    phase, absolute_move = replay_phase(replay["initial_sfen"], position)
    attributes = PROFILES.attributes(position["pre_move_sfen"])
    gives_check = None
    if index + 1 < len(replay["positions"]):
        gives_check = replay["positions"][index + 1].get("side_to_move_in_check") is True
    in_check = position.get("side_to_move_in_check") is True
    danger = "both" if in_check and gives_check else "in_check" if in_check else "gives_check" if gives_check else "none" if gives_check is not None else "unknown"
    source_key = f"{game['source_id']}:{game['game_number']}"
    return {
        "schema_version": 1,
        "sfen": position["pre_move_sfen"],
        "source": {
            "kind": "selfplay-replay",
            "path": game["replay_path"],
            "sha256": game["replay_sha256"],
            "source_id": game["source_id"],
            "game_number": game["game_number"],
            "source_key": source_key,
            "ply": int(position["ply"]),
            "opening": game["opening"],
        },
        "tags": {
            "phase": phase,
            "material_band": attributes["material_band"],
            "material_stm_cp": attributes["material_stm_cp"],
            "mate_status": mate_status(position),
            "king_danger": danger,
            "in_check": in_check,
            "gives_check": gives_check,
        },
        "audit": {
            "absolute_move_number": absolute_move,
            "phase_source": "replay initial move number + replay ply, checked against position SFEN",
        },
    }


class UnionFind:
    def __init__(self, values: Iterable[str]) -> None:
        self.parent = {value: value for value in values}

    def find(self, value: str) -> str:
        parent = self.parent[value]
        if parent != value:
            self.parent[value] = self.find(parent)
        return self.parent[value]

    def union(self, left: str, right: str) -> None:
        left_root, right_root = self.find(left), self.find(right)
        if left_root != right_root:
            keep, merge = sorted((left_root, right_root))
            self.parent[merge] = keep


def load_candidates(ledger: dict[str, Any]) -> tuple[list[dict[str, Any]], dict[str, list[str]], dict[str, Any]]:
    games = ledger.get("representative_games")
    if ledger.get("schema") != "sekirei.selfplay-representative-ledger.v1" or not isinstance(games, list):
        raise ValueError("unsupported representative ledger")
    source_keys = [f"{game['source_id']}:{game['game_number']}" for game in games]
    union = UnionFind(source_keys)
    owner_by_identity: dict[str, str] = {}
    replay_cache: dict[str, dict[str, Any]] = {}
    audit = Counter()
    for game, source_key in zip(games, source_keys):
        replay_path = Path(game["replay_path"])
        replay = json.loads(replay_path.read_text(encoding="utf-8"))
        replay_cache[source_key] = replay
        identities = {symmetry_key(game["opening"])}
        for row in replay.get("positions", []):
            identities.add(symmetry_key(row["pre_move_sfen"]))
        for identity in identities:
            prior = owner_by_identity.get(identity)
            if prior is not None:
                union.union(source_key, prior)
                audit["cross_game_position_or_opening_links"] += 1
            else:
                owner_by_identity[identity] = source_key
    groups: dict[str, list[str]] = defaultdict(list)
    for source_key in source_keys:
        groups[union.find(source_key)].append(source_key)
    group_id_by_source = {
        source_key: hashlib.sha256("\0".join(sorted(members)).encode()).hexdigest()[:16]
        for members in groups.values()
        for source_key in members
    }
    candidates: list[dict[str, Any]] = []
    for game, source_key in zip(games, source_keys):
        replay = replay_cache[source_key]
        for index in range(len(replay.get("positions", []))):
            row = position_row(game, replay, index)
            row["source"]["derived_group"] = group_id_by_source[source_key]
            row["identity"] = symmetry_key(row["sfen"])
            candidates.append(row)
    grouped = defaultdict(list)
    for source_key, group_id in group_id_by_source.items():
        grouped[group_id].append(source_key)
    audit.update({"games": len(games), "derived_groups": len(grouped), "positions": len(candidates)})
    return candidates, dict(grouped), dict(audit)


def _json_documents(path: Path) -> Iterable[Any]:
    if path.suffix == ".jsonl":
        for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
            if line.strip():
                try:
                    yield json.loads(line)
                except json.JSONDecodeError:
                    continue
    elif path.suffix == ".json":
        try:
            yield json.loads(path.read_text(encoding="utf-8", errors="replace"))
        except json.JSONDecodeError:
            return


def _extract_sfens(value: Any) -> Iterable[str]:
    if isinstance(value, str):
        try:
            yield canonical_sfen(value)
        except (ValueError, IndexError):
            return
    elif isinstance(value, dict):
        for child in value.values():
            yield from _extract_sfens(child)
    elif isinstance(value, list):
        for child in value:
            yield from _extract_sfens(child)


def exclusion_files(root: Path) -> list[Path]:
    if root.is_file():
        return [root] if root.suffix in SFEN_SUFFIXES else []
    return sorted(path for path in root.rglob("*") if path.is_file() and path.suffix in SFEN_SUFFIXES)


def load_exclusions(roots: list[Path]) -> tuple[set[str], list[dict[str, Any]]]:
    identities: set[str] = set()
    records: list[dict[str, Any]] = []
    for root in roots:
        files = exclusion_files(root)
        root_identities: set[str] = set()
        file_records = []
        for path in files:
            found: set[str] = set()
            if path.suffix == ".sfen":
                for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
                    try:
                        found.add(symmetry_key(line.strip()))
                    except (ValueError, IndexError):
                        continue
            else:
                for document in _json_documents(path):
                    for sfen in _extract_sfens(document):
                        found.add(symmetry_key(sfen))
            if found:
                file_records.append({"path": str(path), "sha256": sha256(path), "identities": len(found)})
                root_identities.update(found)
        identities.update(root_identities)
        records.append({
            "path": str(root),
            "files_with_positions": len(file_records),
            "symmetry_identities": len(root_identities),
            "files": file_records,
        })
    return identities, records


def assign_groups(
    groups: dict[str, list[str]], rows: list[dict[str, Any]], seed: int, validation_fraction: int = 5
) -> dict[str, str]:
    """Assign whole derived groups while giving validation all nine strata.

    The assignment is label-blind: only phase/material sampling tags and source
    identities are considered.  A deterministic greedy pass first covers the
    documented validation minima, then a hash rank fills roughly one fifth of
    the groups.  This prevents a rare stratum from disappearing merely because
    a plain hash split placed all of its sources on the training side.
    """
    by_group: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in rows:
        by_group[row["source"]["derived_group"]].append(row)
    selected: set[str] = set()
    position_counts = Counter()
    source_sets: dict[str, set[str]] = defaultdict(set)

    def improvement(group_id: str) -> tuple[int, int]:
        positions = Counter(
            f"{row['tags']['phase']}/{row['tags']['material_band']}"
            for row in by_group.get(group_id, [])
        )
        sources: dict[str, set[str]] = defaultdict(set)
        for row in by_group.get(group_id, []):
            stratum = f"{row['tags']['phase']}/{row['tags']['material_band']}"
            sources[stratum].add(row["source"]["source_key"])
        position_gain = sum(
            min(max(16 - position_counts[stratum], 0), positions[stratum]) for stratum in STRATA
        )
        source_gain = sum(
            min(max(4 - len(source_sets[stratum]), 0), len(sources[stratum] - source_sets[stratum]))
            for stratum in STRATA
        )
        return source_gain, position_gain

    while True:
        ranked = [
            (*improvement(group_id), stable_rank(group_id, seed), group_id)
            for group_id in groups
            if group_id not in selected
        ]
        if not ranked:
            break
        source_gain, position_gain, _, group_id = min(
            ranked, key=lambda item: (-item[0], -item[1], item[2])
        )
        if source_gain == 0 and position_gain == 0:
            break
        selected.add(group_id)
        for row in by_group.get(group_id, []):
            stratum = f"{row['tags']['phase']}/{row['tags']['material_band']}"
            position_counts[stratum] += 1
            source_sets[stratum].add(row["source"]["source_key"])

    target = max(len(selected), max(1, round(len(groups) / validation_fraction)))
    for group_id in sorted(groups, key=lambda value: stable_rank(value, seed)):
        if len(selected) >= target:
            break
        selected.add(group_id)
    return {group_id: "validation" if group_id in selected else "train" for group_id in groups}


def select(rows: list[dict[str, Any]], limit: int, source_cap: int, seed: int) -> list[dict[str, Any]]:
    buckets: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in rows:
        stratum = f"{row['tags']['phase']}/{row['tags']['material_band']}"
        buckets[stratum].append(row)
    for bucket in buckets.values():
        bucket.sort(key=lambda row: stable_rank(f"{row['source']['source_key']}\0{row['sfen']}", seed))
    cursors = Counter()
    per_source = Counter()
    selected: list[dict[str, Any]] = []
    selected_identities: set[str] = set()
    while len(selected) < limit:
        progressed = False
        for stratum in STRATA:
            bucket = buckets.get(stratum, [])
            while cursors[stratum] < len(bucket):
                row = bucket[cursors[stratum]]
                cursors[stratum] += 1
                source = row["source"]["source_key"]
                if per_source[source] >= source_cap or row["identity"] in selected_identities:
                    continue
                selected.append(row)
                per_source[source] += 1
                selected_identities.add(row["identity"])
                progressed = True
                break
            if len(selected) >= limit:
                break
        if not progressed:
            break
    return selected


def summarize(rows: list[dict[str, Any]]) -> dict[str, Any]:
    strata = Counter(f"{row['tags']['phase']}/{row['tags']['material_band']}" for row in rows)
    source_sets = {
        stratum: {row["source"]["source_key"] for row in rows if f"{row['tags']['phase']}/{row['tags']['material_band']}" == stratum}
        for stratum in STRATA
    }
    return {
        "positions": len(rows),
        "sources": len({row["source"]["source_key"] for row in rows}),
        "derived_groups": len({row["source"]["derived_group"] for row in rows}),
        "phase_material_positions": {stratum: strata[stratum] for stratum in STRATA},
        "phase_material_sources": {stratum: len(source_sets[stratum]) for stratum in STRATA},
        "mate_status": dict(sorted(Counter(row["tags"]["mate_status"] for row in rows).items())),
        "king_danger": dict(sorted(Counter(row["tags"]["king_danger"] for row in rows).items())),
        "maximum_positions_per_source": max(Counter(row["source"]["source_key"] for row in rows).values(), default=0),
    }


def coverage(summary: dict[str, Any], *, train: bool) -> dict[str, Any]:
    failures = []
    for stratum in STRATA:
        minimum_positions = 32 if train and stratum == "endgame/balanced" else 16
        minimum_sources = 8 if train and stratum == "endgame/balanced" else 4
        positions = summary["phase_material_positions"][stratum]
        sources = summary["phase_material_sources"][stratum]
        if positions < minimum_positions or sources < minimum_sources:
            failures.append({
                "stratum": stratum,
                "positions": positions,
                "required_positions": minimum_positions,
                "sources": sources,
                "required_sources": minimum_sources,
            })
    return {"passed": not failures, "failures": failures}


def write_jsonl(path: Path, rows: list[dict[str, Any]]) -> None:
    cleaned = []
    for row in rows:
        item = dict(row)
        item.pop("identity", None)
        cleaned.append(item)
    path.write_text("".join(json.dumps(row, ensure_ascii=False, sort_keys=True) + "\n" for row in cleaned), encoding="utf-8")


def freeze(
    ledger_path: Path,
    exclusion_roots: list[Path],
    output_dir: Path,
    *,
    train_max: int,
    validation_max: int,
    source_cap: int,
    seed: int,
) -> dict[str, Any]:
    if source_cap <= 0 or source_cap > 16:
        raise ValueError("source_cap must be in 1..=16")
    if train_max <= 0 or train_max > 1024 or validation_max <= 0 or validation_max > 256:
        raise ValueError("train_max must be <=1024 and validation_max <=256")
    ledger = json.loads(ledger_path.read_text(encoding="utf-8"))
    candidates, groups, source_audit = load_candidates(ledger)
    excluded, exclusion_ledger = load_exclusions(exclusion_roots)
    eligible = [row for row in candidates if row["identity"] not in excluded]
    assignments = assign_groups(groups, eligible, seed)
    train_rows = select(
        [row for row in eligible if assignments[row["source"]["derived_group"]] == "train"],
        train_max,
        source_cap,
        seed,
    )
    validation_rows = select(
        [row for row in eligible if assignments[row["source"]["derived_group"]] == "validation"],
        validation_max,
        source_cap,
        seed + 1,
    )
    train_ids = {symmetry_key(row["sfen"]) for row in train_rows}
    validation_ids = {symmetry_key(row["sfen"]) for row in validation_rows}
    train_groups = {row["source"]["derived_group"] for row in train_rows}
    validation_groups = {row["source"]["derived_group"] for row in validation_rows}
    violations = {
        "train_validation_symmetry_overlap": len(train_ids & validation_ids),
        "train_validation_derived_group_overlap": len(train_groups & validation_groups),
        "train_exclusion_overlap": len(train_ids & excluded),
        "validation_exclusion_overlap": len(validation_ids & excluded),
    }
    train_summary, validation_summary = summarize(train_rows), summarize(validation_rows)
    train_coverage = coverage(train_summary, train=True)
    validation_coverage = coverage(validation_summary, train=False)
    valid = not any(violations.values()) and bool(train_rows) and bool(validation_rows)
    output_dir.mkdir(parents=True, exist_ok=True)
    train_path, validation_path = output_dir / "train.jsonl", output_dir / "validation.jsonl"
    write_jsonl(train_path, train_rows)
    write_jsonl(validation_path, validation_rows)
    (output_dir / "exclusions.json").write_text(json.dumps(exclusion_ledger, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    manifest = {
        "schema": "sekirei.q21h-independent-learning-split.v1",
        "status": "frozen_ready" if valid and train_coverage["passed"] and validation_coverage["passed"] else "frozen_hold",
        "strength_claim": False,
        "contract": {
            "phase": "absolute move = replay initial SFEN move + replay ply; checked against each position SFEN; opening<=20, middlegame<=60, endgame>60",
            "material": "side-to-move board+hand material; balanced iff abs(score)<=200cp",
            "mate_status": "mate/non_mate only from an explicit score-kind field; legacy numeric cp is unknown",
            "king_danger": "in_check and next-position in_check (actual move gives check); last-position gives_check is unknown",
            "independence": "connected components over exact, file-mirrored, color-rotated, and mirrored-color-rotated opening/position SFEN identities",
            "source_cap": source_cap,
            "train_max": train_max,
            "validation_max": validation_max,
            "seed": seed,
        },
        "inputs": {
            "ledger": str(ledger_path),
            "ledger_sha256": sha256(ledger_path),
            "exclusion_roots": [str(path) for path in exclusion_roots],
        },
        "audit": {
            **source_audit,
            "eligible_positions": len(eligible),
            "excluded_position_records": len(candidates) - len(eligible),
            "excluded_symmetry_identities": len(excluded),
            "violations": violations,
        },
        "train": {"path": str(train_path), "sha256": sha256(train_path), **train_summary, "coverage": train_coverage},
        "validation": {"path": str(validation_path), "sha256": sha256(validation_path), **validation_summary, "coverage": validation_coverage},
        "selection_ready": valid and train_coverage["passed"] and validation_coverage["passed"],
        "next_action": "pilot may start" if valid and train_coverage["passed"] and validation_coverage["passed"] else "collect only the strata listed in coverage.failures; do not train yet",
    }
    manifest_path = output_dir / "manifest.json"
    manifest_path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return manifest


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ledger", type=Path, required=True)
    parser.add_argument("--exclude", type=Path, action="append", default=[])
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--train-max", type=int, default=1024)
    parser.add_argument("--validation-max", type=int, default=256)
    parser.add_argument("--source-cap", type=int, default=16)
    parser.add_argument("--seed", type=int, default=42)
    args = parser.parse_args()
    manifest = freeze(
        args.ledger,
        args.exclude,
        args.output_dir,
        train_max=args.train_max,
        validation_max=args.validation_max,
        source_cap=args.source_cap,
        seed=args.seed,
    )
    print(json.dumps({
        "status": manifest["status"],
        "selection_ready": manifest["selection_ready"],
        "train": manifest["train"]["positions"],
        "validation": manifest["validation"]["positions"],
        "violations": manifest["audit"]["violations"],
    }, sort_keys=True))
    return 0 if manifest["audit"]["violations"] == {key: 0 for key in manifest["audit"]["violations"]} else 1


if __name__ == "__main__":
    raise SystemExit(main())
