#!/usr/bin/env python3
"""Materialize a deduplicated CSA view from a salvage manifest.

The source games remain untouched.  The output directory contains relative
symlinks only for manifest entries explicitly marked usable for CP teaching.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path


def prepare(manifest_path: Path, output_dir: Path) -> dict[str, object]:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    if manifest.get("schema") != "sekirei.selfplay-csa-salvage.v1":
        raise ValueError("unsupported salvage manifest")
    output_dir.mkdir(parents=True, exist_ok=True)
    selected: list[dict[str, object]] = []
    for game in manifest.get("games", []):
        if not isinstance(game, dict) or game.get("usable_for_cp_teacher") is not True:
            continue
        source = game.get("source")
        if not isinstance(source, dict) or not isinstance(source.get("path"), str):
            raise ValueError("usable game is missing source.path")
        source_path = Path(source["path"])
        if not source_path.is_file():
            raise ValueError(f"missing source CSA: {source_path}")
        game_number = game.get("game_number")
        if not isinstance(game_number, int):
            raise ValueError("usable game is missing game_number")
        link = output_dir / f"game{game_number:04d}.csa"
        if link.exists() or link.is_symlink():
            if not link.is_symlink() or link.resolve() != source_path.resolve():
                raise ValueError(f"refusing to replace non-matching output: {link}")
        else:
            os.symlink(os.path.relpath(source_path, start=output_dir), link)
        selected.append({
            "game_number": game_number,
            "split": game.get("split"),
            "source": str(source_path),
            "source_sha256": source.get("sha256"),
            "link": str(link),
        })
    document = {
        "schema": "sekirei.salvaged-csa-dataset.v1",
        "source_manifest": str(manifest_path),
        "source_status": manifest.get("status"),
        "games": selected,
        "games_total": len(selected),
        "split_counts": {
            "train": sum(game["split"] == "train" for game in selected),
            "validation": sum(game["split"] == "validation" for game in selected),
        },
        "storage": "relative_symlinks",
        "claims": {"deduplicated": True, "strength": "not_established"},
    }
    (output_dir / "dataset.manifest.json").write_text(
        json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    return document


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--salvage-manifest", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    document = prepare(args.salvage_manifest, args.output_dir)
    print(f"prepared {document['games_total']} deduplicated CSA links in {args.output_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
