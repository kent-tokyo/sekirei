#!/usr/bin/env python3
"""Attach a non-strength search diagnostic to a release manifest copy."""
import argparse
import json
from pathlib import Path

from validate_release_manifest import validate


COUNTERS = (
    "tt_probes",
    "tt_hits",
    "order_tt",
    "order_killer",
    "order_countermove",
    "order_history",
)


def record(release_path: Path, output_path: Path, budget: str, depth: int, nodes: int, **counters: int) -> dict:
    release = json.loads(release_path.read_text())
    errors = validate(release)
    if errors:
        raise ValueError("invalid release manifest: " + ", ".join(errors))
    if not budget:
        raise ValueError("search diagnostic budget must not be empty")
    values = {key: counters[key] for key in COUNTERS}
    if min(depth, nodes, *values.values()) < 0:
        raise ValueError("search diagnostic counts must be non-negative")
    release["search_diagnostic"] = {
        "schema": "sekirei.search-diagnostic.v1",
        "budget": budget,
        "depth": depth,
        "nodes": nodes,
        **values,
        "strength_claim": False,
    }
    output_path.write_text(json.dumps(release, indent=2) + "\n")
    return release


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--release-manifest", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--budget", required=True)
    parser.add_argument("--depth", type=int, required=True)
    parser.add_argument("--nodes", type=int, required=True)
    for key in COUNTERS:
        parser.add_argument(f"--{key.replace('_', '-')}", type=int, required=True)
    args = parser.parse_args()
    try:
        record(
            args.release_manifest,
            args.output,
            args.budget,
            args.depth,
            args.nodes,
            **{key: getattr(args, key) for key in COUNTERS},
        )
    except (OSError, json.JSONDecodeError, ValueError) as exc:
        parser.error(str(exc))
    print(f"search diagnostic manifest copy written: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
