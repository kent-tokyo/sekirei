#!/usr/bin/env python3
"""Freeze the inputs and protocol for a future local strength gate."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

from validate_nnue_output_metadata import validate as validate_nnue_output


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def evaluation_binding(weights: Path, requested_mode: str) -> dict[str, object]:
    """Freeze a weight file together with the semantics of its output.

    The binary layout deliberately does not encode whether it is an absolute
    evaluator or a material residual.  Requiring both the caller's explicit
    mode and a validated sidecar keeps a gate from silently falling back to
    the USI default (`absolute`).
    """
    verified = validate_nnue_output(weights, requested_mode)
    metadata = Path(str(verified["metadata"]))
    sidecar = json.loads(metadata.read_text(encoding="utf-8"))
    return {
        "weights": {"path": str(weights), "sha256": sha256(weights)},
        "metadata": {"path": str(metadata), "sha256": sha256(metadata)},
        "mode": verified["mode"],
        "baseline": sidecar.get("baseline"),
        "checkpoint_hash": verified["hash"],
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--openings", type=Path, required=True)
    parser.add_argument("--calibration", type=Path, required=True)
    parser.add_argument("--candidate-nnue-output", required=True,
                        choices=("absolute", "residual-material"))
    parser.add_argument("--baseline-nnue-output", required=True,
                        choices=("absolute", "residual-material"))
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--games-per-position", type=int, default=2)
    parser.add_argument("--byoyomi-ms", type=int, default=1000)
    parser.add_argument("--max-games", type=int, default=400)
    args = parser.parse_args()
    for name, path in (("candidate", args.candidate), ("baseline", args.baseline), ("openings", args.openings), ("calibration", args.calibration)):
        if not path.is_file():
            raise SystemExit(f"missing {name}: {path}")
    if min(args.games_per_position, args.byoyomi_ms, args.max_games) <= 0:
        raise SystemExit("game, byoyomi, and max-game values must be positive")

    positions = [line for line in args.openings.read_text(encoding="utf-8").splitlines() if line.strip() and not line.startswith("#")]
    # A formal gate is deliberately one colour-reversed pair per starting
    # position.  More games per position look like extra sample size but would
    # give a single opening disproportionate weight; fewer loses the colour
    # control.  Keep the contract here, at the single source of truth, rather
    # than letting a runner silently choose a different interpretation.
    if args.games_per_position != 2:
        raise SystemExit("formal strength gates require exactly 2 games per position")
    if len(positions) != 200:
        raise SystemExit(f"formal strength gates require 200 opening positions, got {len(positions)}")
    if args.max_games != len(positions) * args.games_per_position:
        raise SystemExit(
            "formal strength-gate max-games must equal openings × games-per-position "
            f"({len(positions) * args.games_per_position})"
        )

    try:
        candidate_evaluation = evaluation_binding(args.candidate, args.candidate_nnue_output)
        baseline_evaluation = evaluation_binding(args.baseline, args.baseline_nnue_output)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        raise SystemExit(f"invalid NNUE evaluation binding: {error}") from error

    manifest = {
        "schema": "sekirei.strength-gate-plan.v1",
        "status": "planned",
        "strength_claim": False,
        "candidate": candidate_evaluation["weights"],
        "baseline": baseline_evaluation["weights"],
        "evaluation": {
            "candidate": candidate_evaluation,
            "baseline": baseline_evaluation,
        },
        "openings": {"path": str(args.openings), "sha256": sha256(args.openings), "positions": len(positions)},
        "calibration": {"path": str(args.calibration), "sha256": sha256(args.calibration)},
        "protocol": {
            "games_per_position": args.games_per_position,
            "positions": len(positions),
            "byoyomi_ms": args.byoyomi_ms,
            "max_games": args.max_games,
            "engine_options": {"Threads": "1", "SpecTopN": "0", "UseBook": "false", "SearchMode": "Speculative"},
            "sprt": {
                "elo0": 0,
                "elo1": 20,
                "alpha": 0.05,
                "beta": 0.05,
                "variant": "trinomial",
                "paired_by_id": True,
            },
        },
        "resource_policy": {"preflight_required": True, "do_not_infer_from_refusal": True},
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(manifest, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
