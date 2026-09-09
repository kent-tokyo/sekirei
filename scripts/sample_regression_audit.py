#!/usr/bin/env python3
"""Build a deterministic, bounded audit corpus from candidate-loss positions.

The result is intended for NNUE/label diagnostics, not for training or a
strength claim.  Sampling is stratified by the fields already produced by
``summarize_regression_positions.py`` and preserves the first stable rows in
each bucket so reruns are byte-for-byte reproducible.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter
from pathlib import Path


KEYS = ("phase", "legal_move_band", "side_to_move", "hand", "promotion")


def bucket(row: dict[str, object]) -> tuple[str, ...]:
    return tuple(str(row.get(key, "unknown")) for key in KEYS)


def stable_key(row: dict[str, object]) -> tuple[int, int, str]:
    return (int(row.get("game_num", 0)), int(row.get("seq", 0)), str(row["sfen"]))


def sample(rows: list[dict[str, object]], per_bucket: int, limit: int) -> list[dict[str, object]]:
    groups: dict[tuple[str, ...], list[dict[str, object]]] = {}
    for row in rows:
        groups.setdefault(bucket(row), []).append(row)
    selected: list[dict[str, object]] = []
    for key in sorted(groups):
        selected.extend(sorted(groups[key], key=stable_key)[:per_bucket])
    selected.sort(key=stable_key)
    return selected[:limit]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("corpus", type=Path)
    parser.add_argument("--output", type=Path, required=True, help="plain SFEN output")
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--per-bucket", type=int, default=16)
    parser.add_argument("--limit", type=int, default=512)
    args = parser.parse_args()
    if args.per_bucket <= 0 or args.limit <= 0:
        raise SystemExit("--per-bucket and --limit must be positive")

    rows = [json.loads(line) for line in args.corpus.read_text(encoding="utf-8").splitlines() if line.strip()]
    if not rows:
        raise SystemExit("empty audit corpus")
    selected = sample(rows, args.per_bucket, args.limit)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text("\n".join(str(row["sfen"]) for row in selected) + "\n", encoding="utf-8")
    counts = Counter("/".join(bucket(row)) for row in selected)
    source_hash = hashlib.sha256(args.corpus.read_bytes()).hexdigest()
    sample_hash = hashlib.sha256(args.output.read_bytes()).hexdigest()
    manifest = {
        "purpose": "candidate-loss NNUE audit fixture",
        "training_set": False,
        "strength_claim": False,
        "source": str(args.corpus),
        "source_sha256": source_hash,
        "positions_available": len(rows),
        "positions_selected": len(selected),
        "per_bucket": args.per_bucket,
        "limit": args.limit,
        "bucket_keys": list(KEYS),
        "bucket_counts": dict(sorted(counts.items())),
        "sample_sha256": sample_hash,
    }
    args.manifest.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(manifest, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
