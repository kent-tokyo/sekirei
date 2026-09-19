#!/usr/bin/env python3
"""Build a deterministic, balanced root corpus from a teacher-label cache.

The corpus keeps mate-scale and ordinary CP labels separate, then balances by
SFEN-derived phase and material band.  It is a diagnostic sampling tool, not a
training-data selector and not a strength gate.
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
SPEC = importlib.util.spec_from_file_location(
    "profiles", ROOT / "scripts" / "compare_nnue_root_profiles.py"
)
assert SPEC and SPEC.loader
PROFILES = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PROFILES)

MATE_THRESHOLD = 899_000


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    digest.update(path.read_bytes())
    return digest.hexdigest()


def load_cache(path: Path) -> list[dict[str, Any]]:
    rows = []
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip():
            continue
        value = json.loads(line)
        sfen, score = value.get("sfen"), value.get("score_cp")
        if not isinstance(sfen, str) or not isinstance(score, int):
            raise ValueError(f"cache line {line_number} lacks string sfen or integer score_cp")
        attributes = PROFILES.attributes(sfen)
        rows.append(
            {
                "sfen": sfen,
                "teacher_score_cp": score,
                "teacher_class": "mate" if abs(score) >= MATE_THRESHOLD else "non_mate",
                "phase": attributes["phase"],
                "material_band": attributes["material_band"],
            }
        )
    if not rows:
        raise ValueError("teacher cache has no usable rows")
    return rows


def select(rows: list[dict[str, Any]], per_stratum: int) -> list[dict[str, Any]]:
    buckets: dict[tuple[str, str, str], list[dict[str, Any]]] = defaultdict(list)
    for row in rows:
        buckets[(row["teacher_class"], row["phase"], row["material_band"])].append(row)
    selected = []
    for key in sorted(buckets):
        candidates = sorted(buckets[key], key=lambda row: row["sfen"])
        selected.extend(candidates[:per_stratum])
    return selected


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--teacher-cache", type=Path, required=True)
    parser.add_argument("--per-stratum", type=int, default=4)
    parser.add_argument("--output-sfen", type=Path, required=True)
    parser.add_argument("--output-manifest", type=Path, required=True)
    args = parser.parse_args()
    if args.per_stratum <= 0:
        parser.error("--per-stratum must be positive")
    rows = load_cache(args.teacher_cache)
    selected = select(rows, args.per_stratum)
    args.output_sfen.parent.mkdir(parents=True, exist_ok=True)
    args.output_sfen.write_text(
        "".join(f"{row['sfen']}\n" for row in selected), encoding="utf-8"
    )
    manifest = {
        "schema": "sekirei.teacher-strata-corpus.v2",
        "diagnostic_only": True,
        "strength_claim": False,
        "teacher_cache": str(args.teacher_cache),
        "teacher_cache_sha256": sha256(args.teacher_cache),
        "mate_threshold_cp": MATE_THRESHOLD,
        "per_stratum": args.per_stratum,
        "available_counts": dict(
            sorted(
                Counter(
                    f"{row['teacher_class']}/{row['phase']}/{row['material_band']}"
                    for row in rows
                ).items()
            )
        ),
        "selected_counts": dict(
            sorted(
                Counter(
                    f"{row['teacher_class']}/{row['phase']}/{row['material_band']}"
                    for row in selected
                ).items()
            )
        ),
        "rows": selected,
    }
    args.output_manifest.parent.mkdir(parents=True, exist_ok=True)
    args.output_manifest.write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    print(json.dumps({"selected": len(selected), "selected_counts": manifest["selected_counts"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
