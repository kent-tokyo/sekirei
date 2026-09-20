#!/usr/bin/env python3
"""Freeze a deterministic, exclusion-aware NNUE learning pilot corpus.

The trainer's per-source cap is useful for exploratory runs, but a pilot that
will be compared with another learning-rate setting needs its exact input
written once.  This tool removes canonical SFENs listed in one or more JSONL
files, keeps a stable number of positions per source, and records every input
hash.  It does not create a strength-gate corpus.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
from collections import Counter, defaultdict
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
PROFILE_SPEC = importlib.util.spec_from_file_location(
    "nnue_root_profiles", ROOT / "scripts" / "compare_nnue_root_profiles.py"
)
assert PROFILE_SPEC and PROFILE_SPEC.loader
PROFILES = importlib.util.module_from_spec(PROFILE_SPEC)
PROFILE_SPEC.loader.exec_module(PROFILES)


FNV_OFFSET = 14695981039346656037
FNV_PRIME = 1099511628211
MASK64 = (1 << 64) - 1


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


def fnv1a_seeded(value: str, seed: int) -> int:
    result = FNV_OFFSET
    for byte in value.encode("utf-8"):
        result ^= byte
        result = (result * FNV_PRIME) & MASK64
    return result ^ seed


def read_jsonl(path: Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip():
            continue
        row = json.loads(line)
        if not isinstance(row, dict):
            raise ValueError(f"{path}:{line_number}: object is required")
        rows.append(row)
    return rows


def sfen_set(path: Path) -> set[str]:
    result: set[str] = set()
    for index, row in enumerate(read_jsonl(path), 1):
        sfen = row.get("sfen")
        if not isinstance(sfen, str):
            raise ValueError(f"{path}:{index}: sfen is required")
        result.add(canonical_sfen(sfen))
    return result


def phase_material_stratum(row: dict[str, Any]) -> str:
    """Classify a row with its trainer-consumed phase tag and the same
    hand-aware, side-to-move material model as the root-profile diagnostic.

    This is deliberately a sampling label, not a training target.  Keeping it
    in one shared implementation prevents a pilot corpus from silently using
    the old board-only material classification.
    """
    tags = row.get("tags")
    phase = tags.get("phase") if isinstance(tags, dict) else None
    if phase not in {"opening", "middlegame", "endgame"}:
        raise ValueError("row tags.phase must be opening, middlegame, or endgame")
    attributes = PROFILES.attributes(row["sfen"])
    return f"{phase}/{attributes['material_band']}"


def freeze(
    rows: list[dict[str, Any]],
    excluded: set[str],
    per_source: int,
    seed: int,
    stratify_phase_material: bool = False,
) -> tuple[list[dict[str, Any]], dict[str, int]]:
    if per_source <= 0:
        raise ValueError("per_source must be positive")
    by_source: dict[str, list[tuple[int, dict[str, Any]]]] = defaultdict(list)
    rejected = {"excluded": 0, "duplicate": 0}
    seen: set[str] = set()
    validated: list[tuple[str, str, str, dict[str, Any]]] = []
    for index, row in enumerate(rows, 1):
        sfen = row.get("sfen")
        source = row.get("source")
        path = source.get("path") if isinstance(source, dict) else None
        if not isinstance(sfen, str) or not isinstance(path, str) or not path:
            raise ValueError(f"input row {index}: sfen and source.path are required")
        canonical = canonical_sfen(sfen)
        validated.append((canonical, path, sfen, row))
    # Select a stable representative before source-cap ranking.  A repeated
    # board can appear under different replay paths; retaining the first JSONL
    # line would make the frozen corpus depend on file order.
    for canonical, path, sfen, row in sorted(validated, key=lambda item: item[:3]):
        if canonical in excluded:
            rejected["excluded"] += 1
            continue
        if canonical in seen:
            rejected["duplicate"] += 1
            continue
        seen.add(canonical)
        by_source[path].append((fnv1a_seeded(f"{path}\0{sfen}", seed), row))
    if not stratify_phase_material:
        selected: list[dict[str, Any]] = []
        for source in sorted(by_source):
            selected.extend(row for _, row in sorted(by_source[source], key=lambda item: item[0])[:per_source])
        return selected, rejected

    # Select a deterministic round-robin over the nine phase/material strata,
    # still respecting the per-source cap.  A missing stratum is skipped; no
    # row is duplicated to fill it.  Within each stratum the original seeded
    # ranking and source name fully define selection, so input order cannot
    # affect the result.
    buckets: dict[str, list[tuple[int, str, dict[str, Any]]]] = defaultdict(list)
    for source, candidates in by_source.items():
        for rank, row in candidates:
            buckets[phase_material_stratum(row)].append((rank, source, row))
    for candidates in buckets.values():
        candidates.sort(key=lambda item: (item[0], item[1], item[2]["sfen"]))

    selected = []
    selected_per_source: Counter[str] = Counter()
    seen_rows: set[str] = set()
    cursors = {stratum: 0 for stratum in buckets}
    while True:
        progressed = False
        for stratum in sorted(buckets):
            candidates = buckets[stratum]
            cursor = cursors[stratum]
            while cursor < len(candidates):
                _, source, row = candidates[cursor]
                cursor += 1
                if selected_per_source[source] >= per_source:
                    continue
                canonical = canonical_sfen(row["sfen"])
                if canonical in seen_rows:
                    continue
                selected.append(row)
                selected_per_source[source] += 1
                seen_rows.add(canonical)
                progressed = True
                break
            cursors[stratum] = cursor
        if not progressed:
            break
    return selected, rejected


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, required=True)
    parser.add_argument("--exclude-jsonl", type=Path, action="append", default=[])
    parser.add_argument("--per-source", type=int, required=True)
    parser.add_argument(
        "--stratify-phase-material",
        action="store_true",
        help="round-robin phase x hand-aware side-to-move material strata while preserving the per-source cap",
    )
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    rows = read_jsonl(args.input)
    excluded: set[str] = set()
    for path in args.exclude_jsonl:
        excluded.update(sfen_set(path))
    selected, rejected = freeze(
        rows,
        excluded,
        args.per_source,
        args.seed,
        stratify_phase_material=args.stratify_phase_material,
    )
    if not selected:
        parser.error("selection is empty")
    args.output_dir.mkdir(parents=True, exist_ok=True)
    output = args.output_dir / "positions.jsonl"
    output.write_text("".join(json.dumps(row, ensure_ascii=False, sort_keys=True) + "\n" for row in selected), encoding="utf-8")
    manifest = {
        "schema": "sekirei.nnue-learning-pilot-corpus.v1",
        "status": "frozen_development_only",
        "strength_claim": False,
        "selection": {
            "algorithm": (
                "round-robin phase/material strata, then FNV-1a(source.path + NUL + full SFEN) XOR seed"
                if args.stratify_phase_material
                else "FNV-1a(source.path + NUL + full SFEN) XOR seed, lowest per source"
            ),
            "seed": args.seed,
            "per_source": args.per_source,
            "stratify_phase_material": args.stratify_phase_material,
        },
        "input": {"path": str(args.input), "sha256": sha256(args.input), "rows": len(rows)},
        "exclusions": [{"path": str(path), "sha256": sha256(path), "canonical_sfen_count": len(sfen_set(path))} for path in args.exclude_jsonl],
        "result": {
            "path": str(output),
            "sha256": sha256(output),
            "positions": len(selected),
            "sources": len({row["source"]["path"] for row in selected}),
            "phase_counts": dict(sorted(Counter(row.get("tags", {}).get("phase", "missing") for row in selected).items())),
            "phase_material_counts": dict(sorted(Counter(phase_material_stratum(row) for row in selected).items())),
            "rejected": rejected,
        },
    }
    (args.output_dir / "manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"positions": len(selected), "sources": manifest["result"]["sources"], **rejected}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
