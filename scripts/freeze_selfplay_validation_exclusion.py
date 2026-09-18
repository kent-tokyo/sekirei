#!/usr/bin/env python3
"""Seal a local self-play corpus as development-only validation exclusions.

The output is intentionally not a train/validation split.  Every opening
group and every recorded pre-move position in the supplied self-play source is
listed as ineligible for future independent validation.  It also records
overlap with an already frozen training/hold-out split without changing that
split.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def canonical_sfen(sfen: str) -> str:
    fields = sfen.split()
    if len(fields) < 3:
        raise ValueError(f"invalid SFEN: {sfen!r}")
    return " ".join(fields[:3])


def dataset_positions(path: Path) -> set[str]:
    result: set[str] = set()
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip():
            continue
        row = json.loads(line)
        sfen = row.get("sfen")
        if not isinstance(sfen, str):
            raise ValueError(f"{path}:{line_number}: sfen is required")
        result.add(canonical_sfen(sfen))
    return result


def resolve(path: str, relative_to: Path) -> Path:
    candidate = Path(path)
    return candidate if candidate.is_absolute() else relative_to / candidate


def build(review_manifest: Path, train_positions: Path, holdout_positions: Path) -> tuple[dict, list[dict]]:
    review = json.loads(review_manifest.read_text(encoding="utf-8"))
    if review.get("schema") != "sekirei.selfplay-csa-salvage.v1":
        raise ValueError("review manifest must be sekirei.selfplay-csa-salvage.v1")
    source_run = review.get("source_run")
    if not isinstance(source_run, str):
        raise ValueError("review manifest has no source_run")
    source_run_path = resolve(source_run, ROOT)
    run_manifest = source_run_path / "run-manifest.json"
    if not run_manifest.is_file():
        raise ValueError(f"source run manifest missing: {run_manifest}")
    games = review.get("games")
    if not isinstance(games, list):
        raise ValueError("review manifest games must be a list")
    openings: set[str] = set()
    positions: set[str] = set()
    rows: list[dict] = []
    for game in games:
        initial = game.get("initial_sfen")
        replay = game.get("replay", {})
        replay_path = replay.get("path") if isinstance(replay, dict) else None
        if not isinstance(initial, str) or not isinstance(replay_path, str):
            raise ValueError("game lacks initial_sfen or replay path")
        opening = canonical_sfen(initial)
        openings.add(opening)
        path = resolve(replay_path, ROOT)
        document = json.loads(path.read_text(encoding="utf-8"))
        for position in document.get("positions", []):
            sfen = position.get("pre_move_sfen")
            if not isinstance(sfen, str):
                raise ValueError(f"{path}: position lacks pre_move_sfen")
            canonical = canonical_sfen(sfen)
            if canonical in positions:
                continue
            positions.add(canonical)
            rows.append({"sfen": canonical, "status": "development_only_excluded", "source_opening": opening})
    train = dataset_positions(train_positions)
    holdout = dataset_positions(holdout_positions)
    document = {
        "schema": "sekirei.selfplay-validation-exclusion.v1",
        "status": "sealed_development_only",
        "strength_claim": False,
        "rule": "No listed opening group or pre-move position may be used as independent validation or strength-gate evidence.",
        "source": {
            "review_manifest": str(review_manifest), "review_manifest_sha256": sha256(review_manifest),
            "source_run_manifest": str(run_manifest), "source_run_manifest_sha256": sha256(run_manifest),
            "completed_games": len(games),
        },
        "exclusions": {"opening_groups": sorted(openings), "pre_move_positions_file": "development-only-positions.jsonl", "pre_move_position_count": len(positions)},
        "reference_split": {
            "train_positions": {"path": str(train_positions), "sha256": sha256(train_positions), "count": len(train), "overlap_excluded_positions": len(positions & train)},
            "holdout_positions": {"path": str(holdout_positions), "sha256": sha256(holdout_positions), "count": len(holdout), "overlap_excluded_positions": len(positions & holdout)},
            "overlap_between_reference_train_and_holdout": len(train & holdout),
        },
    }
    return document, sorted(rows, key=lambda row: row["sfen"])


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--review-manifest", type=Path, required=True)
    parser.add_argument("--train-positions", type=Path, required=True)
    parser.add_argument("--holdout-positions", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    document, rows = build(args.review_manifest, args.train_positions, args.holdout_positions)
    args.output_dir.mkdir(parents=True, exist_ok=True)
    (args.output_dir / "validation-exclusion-ledger.json").write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    with (args.output_dir / "development-only-positions.jsonl").open("w", encoding="utf-8") as output:
        for row in rows:
            output.write(json.dumps(row, ensure_ascii=False, sort_keys=True) + "\n")
    print(json.dumps({"opening_groups": len(document["exclusions"]["opening_groups"]), "positions": len(rows), "train_overlap": document["reference_split"]["train_positions"]["overlap_excluded_positions"], "holdout_overlap": document["reference_split"]["holdout_positions"]["overlap_excluded_positions"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
