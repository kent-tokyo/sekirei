#!/usr/bin/env python3
"""Split diagnostic root-ranking pairs by connected provenance groups.

The input is intentionally a legal-move *prefix* corpus, not policy truth.
This tool only creates a reproducible train/validation boundary for ranking
loss diagnostics; it does not select a strength candidate.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
from collections import defaultdict
from pathlib import Path


SCHEMA = "sekirei.root-rank-pairs.v1"
SPLIT_SCHEMA = "sekirei.root-rank-pair-split.v1"


def canonical_sfen(sfen: str) -> str:
    """Ignore SFEN's ply counter, which is not part of a board position."""
    fields = sfen.split()
    if len(fields) < 3:
        raise ValueError("SFEN must include board, side, and hands")
    return " ".join(fields[:3])


def game_key(pair: dict) -> str:
    """Return a stable source-game key without trusting a display-only ID."""
    source = pair.get("source")
    if isinstance(source, dict):
        for key in ("replay_sha256", "game_sha256", "game_id"):
            value = source.get(key)
            if isinstance(value, str) and value:
                return f"source:{key}:{value}"
        replay = source.get("replay")
        if isinstance(replay, str) and replay:
            return f"source:replay:{Path(replay).stem}"
    identifier = pair["parent_id"]
    # Existing artifacts predate `source`; their IDs conventionally end in
    # -plyNNN.  Keep the whole game together, but advertise this fallback in
    # the manifest instead of claiming a source hash.
    return f"legacy-id:{re.sub(r'-ply\d+$', '', identifier)}"


def prefix_key(pair: dict) -> str:
    history = pair.get("history_before_usi")
    if not isinstance(history, list) or not all(isinstance(move, str) and move for move in history):
        raise ValueError("pair has no usable history_before_usi")
    initial = pair.get("initial_sfen")
    if not isinstance(initial, str) or not initial:
        raise ValueError("pair has no usable initial_sfen")
    return f"prefix:{canonical_sfen(initial)}\0{' '.join(history)}"


def bucket(component: tuple[str, ...], seed: int) -> int:
    joined = "\0".join(component)
    value = f"{seed}\0{joined}".encode()
    return int.from_bytes(hashlib.sha256(value).digest()[:8], "big") % 1000


def document_sha256(document: dict) -> str:
    encoded = json.dumps(document, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(encoded).hexdigest()


def components(pairs: list[dict]) -> tuple[dict[int, tuple[str, ...]], dict[str, int]]:
    """Join pairs sharing a source game, replay prefix, or board position."""
    parent = list(range(len(pairs)))

    def find(index: int) -> int:
        while parent[index] != index:
            parent[index] = parent[parent[index]]
            index = parent[index]
        return index

    def union(left: int, right: int) -> None:
        left, right = find(left), find(right)
        if left != right:
            parent[right] = left

    owners: dict[str, int] = {}
    for index, pair in enumerate(pairs):
        keys = (
            game_key(pair),
            prefix_key(pair),
            f"position:{canonical_sfen(pair['parent_sfen'])}",
        )
        for key in keys:
            previous = owners.setdefault(key, index)
            union(index, previous)

    members: dict[int, list[int]] = defaultdict(list)
    for index in range(len(pairs)):
        members[find(index)].append(index)
    labels = {
        root: tuple(sorted(pairs[index]["parent_id"] for index in indices))
        for root, indices in members.items()
    }
    assignment = {index: find(index) for index in range(len(pairs))}
    return labels, assignment


def split(document: dict, validation_per_thousand: int, seed: int,
          holdout_per_thousand: int = 0) -> tuple[dict, dict, dict, dict | None]:
    if document.get("schema") != SCHEMA:
        raise ValueError("unsupported ranking pair schema")
    if document.get("diagnostic_only") is not True or document.get("strength_claim") != "not_permitted":
        raise ValueError("ranking pairs must remain diagnostic-only")
    if not 0 < validation_per_thousand < 1000:
        raise ValueError("validation-per-thousand must be between 1 and 999")
    if not 0 <= holdout_per_thousand < 1000 or validation_per_thousand + holdout_per_thousand >= 1000:
        raise ValueError("validation-per-thousand plus holdout-per-thousand must be between 1 and 999")
    pairs = document.get("pairs")
    if not isinstance(pairs, list) or not pairs:
        raise ValueError("ranking pairs must be a non-empty list")
    for pair in pairs:
        if not isinstance(pair, dict):
            raise ValueError("pair must be an object")
        parent_id, parent_sfen = pair.get("parent_id"), pair.get("parent_sfen")
        if not isinstance(parent_id, str) or not parent_id or not isinstance(parent_sfen, str) or not parent_sfen:
            raise ValueError("pair has no usable parent identity")
    labels, assignment = components(pairs)
    component_split = {}
    for root, label in labels.items():
        value = bucket(label, seed)
        component_split[root] = (
            "valid" if value < validation_per_thousand
            else "holdout" if value < validation_per_thousand + holdout_per_thousand
            else "train"
        )
    train, valid, holdout = [], [], []
    train_components, valid_components, holdout_components = set(), set(), set()
    for index, pair in enumerate(pairs):
        arm = component_split[assignment[index]]
        if arm == "valid":
            valid_components.add(assignment[index])
            valid.append(pair)
        elif arm == "holdout":
            holdout_components.add(assignment[index])
            holdout.append(pair)
        else:
            train_components.add(assignment[index])
            train.append(pair)
    if not train or not valid:
        raise ValueError("split produced an empty arm; expand parent corpus or change seed")
    if holdout_per_thousand and not holdout:
        raise ValueError("split produced an empty holdout; expand parent corpus or change seed")
    if train_components & valid_components or train_components & holdout_components or valid_components & holdout_components:
        raise AssertionError("provenance component leakage across ranking split")
    base = {key: value for key, value in document.items() if key != "pairs"}
    manifest = {
        "schema": SPLIT_SCHEMA,
        "diagnostic_only": True,
        "strength_claim": "not_permitted",
        "input_schema": SCHEMA,
        "seed": seed,
        "validation_per_thousand": validation_per_thousand,
        "holdout_per_thousand": holdout_per_thousand,
        "train_pairs": len(train),
        "valid_pairs": len(valid),
        "holdout_pairs": len(holdout),
        "split_unit": "connected_source_game_prefix_or_position",
        "source_game_key_precedence": ["source.replay_sha256", "source.game_sha256", "source.game_id", "source.replay", "legacy parent_id"],
        "train_components": sum(arm == "train" for arm in component_split.values()),
        "valid_components": sum(arm == "valid" for arm in component_split.values()),
        "holdout_components": sum(arm == "holdout" for arm in component_split.values()),
        "component_leakage": 0,
        "input_sha256": document_sha256(document),
    }
    train_document = {**base, "pairs": train}
    valid_document = {**base, "pairs": valid}
    holdout_document = {**base, "pairs": holdout} if holdout else None
    manifest["train_sha256"] = document_sha256(train_document)
    manifest["valid_sha256"] = document_sha256(valid_document)
    manifest["holdout_sha256"] = document_sha256(holdout_document) if holdout_document else None
    return train_document, valid_document, holdout_document, manifest


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--train-output", type=Path, required=True)
    parser.add_argument("--valid-output", type=Path, required=True)
    parser.add_argument("--manifest-output", type=Path, required=True)
    parser.add_argument("--validation-per-thousand", type=int, default=250)
    parser.add_argument("--holdout-per-thousand", type=int, default=0)
    parser.add_argument("--holdout-output", type=Path)
    parser.add_argument("--seed", type=int, default=42)
    args = parser.parse_args()
    try:
        source = json.loads(args.input.read_text(encoding="utf-8"))
        if bool(args.holdout_output) != bool(args.holdout_per_thousand):
            raise ValueError("--holdout-output and --holdout-per-thousand must be used together")
        train, valid, holdout, manifest = split(
            source, args.validation_per_thousand, args.seed, args.holdout_per_thousand
        )
    except (OSError, ValueError, json.JSONDecodeError) as error:
        parser.error(str(error))
    outputs = [(args.train_output, train), (args.valid_output, valid), (args.manifest_output, manifest)]
    if args.holdout_output and holdout:
        outputs.append((args.holdout_output, holdout))
    for path, document in outputs:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(manifest, ensure_ascii=False, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
