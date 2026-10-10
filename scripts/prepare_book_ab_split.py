#!/usr/bin/env python3
"""Freeze disjoint CSA subsets for opening-book training and held-out gates.

The source files are sorted by name, copied into separate directories, and
identified by SHA-256 in a manifest. A file can belong to exactly one arm.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import shutil
from pathlib import Path


SCHEMA = "sekirei.book-ab-source-split.v1"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def select_sources(source_dir: Path, training_games: int, heldout_games: int) -> tuple[list[Path], list[Path]]:
    sources = sorted(source_dir.glob("*.csa"))
    required = training_games + heldout_games
    if len(sources) < required:
        raise ValueError(f"need {required} CSA files, found {len(sources)}")
    training = sources[:training_games]
    heldout = sources[training_games:required]
    if set(map(Path.resolve, training)) & set(map(Path.resolve, heldout)):
        raise ValueError("training and held-out source sets overlap")
    return training, heldout


def copy_subset(sources: list[Path], destination: Path) -> list[dict]:
    destination.mkdir(parents=True, exist_ok=False)
    rows = []
    for source in sources:
        target = destination / source.name
        shutil.copy2(source, target)
        source_hash = sha256(source)
        if sha256(target) != source_hash:
            raise ValueError(f"copied CSA hash mismatch: {source.name}")
        rows.append({
            "path": str(source.resolve()),
            "name": source.name,
            "sha256": source_hash,
            "bytes": source.stat().st_size,
        })
    return rows


def prepare(
    source_dir: Path,
    output_dir: Path,
    training_games: int,
    heldout_games: int,
    source_manifests: list[Path] | None = None,
) -> dict:
    if training_games <= 0 or heldout_games <= 0:
        raise ValueError("training-games and heldout-games must be positive")
    if output_dir.exists():
        raise ValueError(f"output directory already exists: {output_dir}")
    training, heldout = select_sources(source_dir, training_games, heldout_games)
    output_dir.mkdir(parents=True)
    training_rows = copy_subset(training, output_dir / "training")
    heldout_rows = copy_subset(heldout, output_dir / "heldout")
    training_hashes = {row["sha256"] for row in training_rows}
    heldout_hashes = {row["sha256"] for row in heldout_rows}
    if training_hashes & heldout_hashes:
        raise ValueError("identical CSA content appears in both subsets")
    return {
        "schema": SCHEMA,
        "source_dir": str(source_dir.resolve()),
        "source_manifests": [
            {"path": str(path.resolve()), "sha256": sha256(path)}
            for path in (source_manifests or [])
        ],
        "selection": "lexicographic_filename_prefix",
        "training": {"count": len(training_rows), "sources": training_rows},
        "heldout": {"count": len(heldout_rows), "sources": heldout_rows},
        "disjoint_by_path": True,
        "disjoint_by_sha256": True,
        "strength_claim_permitted": False,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-dir", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--training-games", type=int, default=80)
    parser.add_argument("--heldout-games", type=int, default=20)
    parser.add_argument("--source-manifest", type=Path, action="append", default=[])
    args = parser.parse_args()
    try:
        manifest = prepare(
            args.source_dir,
            args.output_dir,
            args.training_games,
            args.heldout_games,
            args.source_manifest,
        )
    except (OSError, ValueError) as error:
        parser.error(str(error))
    path = args.output_dir / "split_manifest.json"
    path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {path}: {args.training_games} training, {args.heldout_games} held out")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
