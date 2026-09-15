#!/usr/bin/env python3
"""Build a deterministic, deduplicated SFEN corpus from salvaged CSA replay."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def build(manifest_path: Path, output: Path, limit: int) -> dict[str, object]:
    if limit <= 0:
        raise ValueError("limit must be positive")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    if manifest.get("schema") != "sekirei.selfplay-csa-salvage.v1":
        raise ValueError("unsupported salvage manifest")
    rows: list[dict[str, object]] = []
    seen: set[str] = set()
    for game in manifest.get("games", []):
        if not isinstance(game, dict) or game.get("usable_for_cp_teacher") is not True:
            continue
        replay = game.get("replay")
        if not isinstance(replay, dict) or not isinstance(replay.get("path"), str):
            raise ValueError("usable game is missing replay.path")
        positions = json.loads(Path(replay["path"]).read_text(encoding="utf-8")).get("positions", [])
        for position in positions:
            if not isinstance(position, dict) or not isinstance(position.get("pre_move_sfen"), str):
                continue
            sfen = position["pre_move_sfen"]
            if sfen in seen:
                continue
            seen.add(sfen)
            rows.append({
                "sample_id": f"game{int(game['game_number']):04d}_ply{int(position.get('ply', 0)):03d}",
                "sfen": sfen,
                "split": game.get("split"),
                "game_number": game["game_number"],
                "ply": position.get("ply"),
                "selected_from": "deduplicated_salvage",
            })
            if len(rows) == limit:
                break
        if len(rows) == limit:
            break
    if not rows:
        raise ValueError("no usable replay positions")
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text("".join(json.dumps(row, ensure_ascii=False) + "\n" for row in rows), encoding="utf-8")
    return {
        "schema": "sekirei.salvaged-diagnostic-corpus.v1",
        "source_manifest": str(manifest_path),
        "source_manifest_sha256": sha256(manifest_path),
        "output": str(output),
        "output_sha256": sha256(output),
        "positions": len(rows),
        "unique_sfens": len(seen),
        "split_counts": {
            "train": sum(row["split"] == "train" for row in rows),
            "validation": sum(row["split"] == "validation" for row in rows),
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--salvage-manifest", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--limit", type=int, default=32)
    parser.add_argument("--manifest", type=Path, required=True)
    args = parser.parse_args()
    document = build(args.salvage_manifest, args.output, args.limit)
    args.manifest.parent.mkdir(parents=True, exist_ok=True)
    args.manifest.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {document['positions']} positions to {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
