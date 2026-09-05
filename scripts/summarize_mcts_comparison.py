#!/usr/bin/env python3
"""Summarize a validated fixed-budget MCTS comparison without strength claims."""
import argparse
import json
from pathlib import Path

from validate_release_manifest import validate


def summarize(path: Path) -> dict:
    manifest = json.loads(path.read_text())
    errors = validate(manifest)
    if errors:
        raise ValueError("invalid release manifest: " + ", ".join(errors))
    comparison = manifest.get("mcts_comparison")
    if not isinstance(comparison, dict):
        raise ValueError("manifest has no mcts_comparison")
    positions = []
    for position in comparison["positions"]:
        tree = position["tree"]
        shared = position["shared"]
        tree_nodes = tree["nodes"]
        shared_nodes = shared["nodes"]
        reduction = 0.0 if tree_nodes == 0 else (tree_nodes - shared_nodes) * 100.0 / tree_nodes
        score_equal = tree["score"] == shared["score"]
        best_move_equal = tree["best_move"] == shared["best_move"]
        agreement = "exact" if score_equal and best_move_equal else (
            "best_move_only" if best_move_equal else "divergent"
        )
        positions.append(
            {
                "name": position["name"],
                "tree_nodes": tree_nodes,
                "shared_nodes": shared_nodes,
                "node_reduction_percent": round(reduction, 3),
                "transposition_hits": shared["transposition_hits"],
                "score_equal": score_equal,
                "best_move_equal": best_move_equal,
                "agreement": agreement,
            }
        )
    return {
        "schema": "sekirei.mcts-comparison-summary.v1",
        "release": manifest["release"],
        "simulations": comparison["simulations"],
        "max_depth": comparison["max_depth"],
        "repeats": comparison["repeats"],
        "strength_claim": False,
        "positions": positions,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    try:
        summary = summarize(args.manifest)
        rendered = json.dumps(summary, indent=2) + "\n"
        if args.output:
            args.output.write_text(rendered)
            print(f"MCTS comparison summary written: {args.output}")
        else:
            print(rendered, end="")
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        parser.error(str(exc))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
