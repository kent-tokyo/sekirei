#!/usr/bin/env python3
"""Freeze the inputs and protocol for a future local strength gate."""

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


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--openings", type=Path, required=True)
    parser.add_argument("--calibration", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--games-per-position", type=int, default=4)
    parser.add_argument("--byoyomi-ms", type=int, default=1000)
    parser.add_argument("--max-games", type=int, default=400)
    args = parser.parse_args()
    for name, path in (("candidate", args.candidate), ("baseline", args.baseline), ("openings", args.openings), ("calibration", args.calibration)):
        if not path.is_file():
            raise SystemExit(f"missing {name}: {path}")
    if min(args.games_per_position, args.byoyomi_ms, args.max_games) <= 0:
        raise SystemExit("game, byoyomi, and max-game values must be positive")

    positions = [line for line in args.openings.read_text(encoding="utf-8").splitlines() if line.strip() and not line.startswith("#")]
    manifest = {
        "schema": "sekirei.strength-gate-plan.v1",
        "status": "planned",
        "strength_claim": False,
        "candidate": {"path": str(args.candidate), "sha256": sha256(args.candidate)},
        "baseline": {"path": str(args.baseline), "sha256": sha256(args.baseline)},
        "openings": {"path": str(args.openings), "sha256": sha256(args.openings), "positions": len(positions)},
        "calibration": {"path": str(args.calibration), "sha256": sha256(args.calibration)},
        "protocol": {
            "games_per_position": args.games_per_position,
            "byoyomi_ms": args.byoyomi_ms,
            "max_games": args.max_games,
            "engine_options": {"Threads": "1", "SpecTopN": "0", "UseBook": "false", "SearchMode": "Speculative"},
            "sprt": {"elo0": 0, "elo1": 20, "alpha": 0.05, "beta": 0.05, "variant": "wald"},
        },
        "resource_policy": {"preflight_required": True, "do_not_infer_from_refusal": True},
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(manifest, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
