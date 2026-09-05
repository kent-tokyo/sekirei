#!/usr/bin/env python3
"""Attach a verified, non-strength MCTS diagnostic to a manifest copy."""
import argparse
import json
from pathlib import Path

from validate_release_manifest import validate


def record(release_path: Path, output_path: Path, mode: str, simulations: int, arena_nodes: int, transposition_hits: int) -> dict:
    release = json.loads(release_path.read_text())
    errors = validate(release)
    if errors:
        raise ValueError("invalid release manifest: " + ", ".join(errors))
    if min(simulations, arena_nodes, transposition_hits) < 0:
        raise ValueError("MCTS diagnostic counts must be non-negative")
    if mode not in {"TreeMcts", "SharedMcts"}:
        raise ValueError("unsupported MCTS mode: " + mode)
    release["mcts_diagnostic"] = {
        "schema": "sekirei.mcts-diagnostic.v1",
        "mode": mode,
        "simulations": simulations,
        "arena_nodes": arena_nodes,
        "transposition_hits": transposition_hits,
        "strength_claim": False,
    }
    output_path.write_text(json.dumps(release, indent=2) + "\n")
    return release


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--release-manifest", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--mode", choices=("TreeMcts", "SharedMcts"), required=True)
    parser.add_argument("--simulations", type=int, required=True)
    parser.add_argument("--arena-nodes", type=int, required=True)
    parser.add_argument("--transposition-hits", type=int, required=True)
    args = parser.parse_args()
    try:
        record(args.release_manifest, args.output, args.mode, args.simulations, args.arena_nodes, args.transposition_hits)
    except (OSError, json.JSONDecodeError, ValueError) as exc:
        parser.error(str(exc))
    print(f"MCTS diagnostic manifest copy written: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
