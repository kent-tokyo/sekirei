#!/usr/bin/env python3
"""Create a reproducible symlinked CSA subset with source hashes."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def choose(source: Path, count: int, offset: int = 0) -> list[Path]:
    files = sorted(path for path in source.rglob("*.csa") if path.is_file())
    ranked = sorted(files, key=lambda path: hashlib.sha256(str(path).encode()).hexdigest())
    if offset < 0:
        raise ValueError("offset must be non-negative")
    if len(ranked) < offset + count:
        raise ValueError(
            f"{source}: requested {count} CSA files at offset {offset}, found {len(ranked)}"
        )
    return ranked[offset:offset + count]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--count", type=int, required=True)
    parser.add_argument("--offset", type=int, default=0,
                        help="skip this many deterministically ranked source files")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.count <= 0 or args.offset < 0:
        parser.error("--count must be positive and --offset must be non-negative")
    selected = choose(args.source, args.count, args.offset)
    args.output.mkdir(parents=True, exist_ok=True)
    entries = []
    for index, source in enumerate(selected, start=1):
        link = args.output / f"game{index:04}.csa"
        relative = os.path.relpath(source.resolve(), start=args.output.resolve())
        if link.exists() or link.is_symlink():
            if not link.is_symlink() or os.readlink(link) != relative:
                raise ValueError(f"refusing to replace unexpected {link}")
        else:
            os.symlink(relative, link)
        entries.append({"source": str(source), "sha256": sha256(source), "link": str(link)})
    manifest = {
        "schema": "sekirei.hashed-csa-subset.v1",
        "diagnostic_only": True,
        "selection": "sha256(path string) rank range, deterministic and source-order independent",
        "source": str(args.source),
        "offset": args.offset,
        "count": len(entries),
        "entries": entries,
    }
    path = args.output / "dataset.manifest.json"
    path.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {path}: {len(entries)} games")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
