#!/usr/bin/env python3
"""Profile static NNUE output diversity on replayable self-play positions.

This is a calibration diagnostic, not a strength test.  It reports the
distribution of one explicitly named weight file on a fixed transcript and
keeps the evaluator used to create that transcript separate from the probe
evaluator used for reanalysis.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import statistics
import subprocess
from collections import Counter
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
PROFILE_SPEC = importlib.util.spec_from_file_location("profiles", ROOT / "scripts" / "compare_nnue_root_profiles.py")
assert PROFILE_SPEC and PROFILE_SPEC.loader
PROFILES = importlib.util.module_from_spec(PROFILE_SPEC)
PROFILE_SPEC.loader.exec_module(PROFILES)
SWING_SPEC = importlib.util.spec_from_file_location("swing", ROOT / "scripts" / "run_selfplay_swing_diagnostic.py")
assert SWING_SPEC and SWING_SPEC.loader
SWING = importlib.util.module_from_spec(SWING_SPEC)
SWING_SPEC.loader.exec_module(SWING)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def transcript_positions(path: Path, limit: int) -> list[str]:
    values: list[str] = []
    seen: set[str] = set()
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip():
            continue
        row = json.loads(line)
        if row.get("verdict") != "ok":
            continue
        sfen = row.get("sfen_before")
        if not isinstance(sfen, str):
            raise ValueError(f"transcript line {line_number} lacks sfen_before")
        if sfen not in seen:
            seen.add(sfen)
            values.append(sfen)
            if len(values) == limit:
                break
    if not values:
        raise ValueError("transcript has no successful positions")
    return values


def static_score(probe: Path, weights: Path, nnue_output: str, sfen: str) -> int:
    completed = subprocess.run(
        # `--strict` is intentionally omitted for a one-position call: its
        # constant-output guard is meaningful only across the probe's built-in
        # multi-position set.  The caller must retain a separate strict
        # preflight result; this script measures the requested corpus itself.
        [str(probe), str(weights), "--json", "--nnue-output", nnue_output, "--sfen", sfen],
        text=True,
        capture_output=True,
        check=False,
        timeout=30,
    )
    if completed.returncode:
        raise RuntimeError(f"nnue probe failed: {completed.stderr.strip()[-800:]}")
    document = json.loads(completed.stdout)
    probes = document.get("probes")
    if not isinstance(probes, list) or len(probes) != 1 or not isinstance(probes[0].get("score_cp"), int):
        raise ValueError("nnue probe returned no single integer score")
    return probes[0]["score_cp"]


def summarize(rows: list[dict[str, Any]]) -> dict[str, Any]:
    scores = [row["static_score_cp"] for row in rows]
    counts = Counter(scores)
    dominant_score, dominant_count = counts.most_common(1)[0]
    return {
        "positions": len(rows),
        "distinct_scores": len(counts),
        "score_range_cp": max(scores) - min(scores),
        "score_variance_cp2": statistics.pvariance(scores),
        "dominant_score_cp": dominant_score,
        "dominant_score_count": dominant_count,
        "dominant_score_ratio": dominant_count / len(scores),
        "score_counts": [{"score_cp": score, "count": count} for score, count in counts.most_common()],
        "phase_counts": dict(sorted(Counter(row["attributes"]["phase"] for row in rows).items())),
        "material_band_counts": dict(sorted(Counter(row["attributes"]["material_band"] for row in rows).items())),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--transcript", type=Path, required=True)
    parser.add_argument("--probe", type=Path, required=True)
    parser.add_argument("--weights", type=Path, required=True)
    parser.add_argument("--nnue-output", choices=("absolute", "residual-material"), default="absolute")
    parser.add_argument("--limit", type=int, default=128)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.limit <= 0:
        parser.error("--limit must be positive")
    try:
        sfens = transcript_positions(args.transcript, args.limit)
        rows = [
            {"id": f"position-{index:03d}", "sfen": sfen, "attributes": PROFILES.attributes(sfen),
             "static_score_cp": static_score(args.probe, args.weights, args.nnue_output, sfen)}
            for index, sfen in enumerate(sfens, 1)
        ]
        document = {
            "schema": "sekirei.nnue-transcript-profile.v1",
            "diagnostic_only": True,
            "strength_claim": False,
            "contract": {"nnue_output": args.nnue_output, "static_evaluation": True, "limit": args.limit},
            "inputs": {
                "transcript": str(args.transcript),
                "transcript_sha256": sha256(args.transcript),
                "source_evaluator": SWING.source_evaluator(args.transcript),
                "probe": str(args.probe), "probe_sha256": sha256(args.probe),
                "weights": str(args.weights), "weights_sha256": sha256(args.weights),
            },
            "rows": rows,
            "summary": summarize(rows),
        }
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        print(json.dumps(document["summary"], sort_keys=True))
    except (OSError, ValueError, json.JSONDecodeError, RuntimeError) as exc:
        parser.error(str(exc))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
