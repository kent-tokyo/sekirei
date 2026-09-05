#!/usr/bin/env python3
"""Record a deterministic fixed-budget MCTS comparison in a manifest copy."""
import argparse
import json
import re
from pathlib import Path

from validate_release_manifest import validate


LINE = re.compile(
    r"^repeat=(?P<repeat>\d+) position=(?P<position>\S+) mode=(?P<mode>TreeMcts|SharedTreeMcts) "
    r"simulations=(?P<simulations>\d+) max_depth=(?P<max_depth>\d+) nodes=(?P<nodes>\d+) score=(?P<score>-?\d+) "
    r"best_move=(?P<best_move>.+?) (?:value_cache_hits=(?P<cache>\d+)|transposition_hits=(?P<hits>\d+))$",
    re.MULTILINE,
)


def parse_comparison(text: str) -> dict:
    matches = list(LINE.finditer(text))
    if not matches:
        raise ValueError("comparison transcript has no diagnostic lines")
    rows = {}
    for match in matches:
        data = match.groupdict()
        key = (data["position"], data["mode"])
        row = {
            "simulations": int(data["simulations"]),
            "max_depth": int(data["max_depth"]),
            "nodes": int(data["nodes"]),
            "score": int(data["score"]),
            "best_move": data["best_move"],
            "transposition_hits": int(data["hits"] or 0),
        }
        previous = rows.setdefault(key, {"repeats": set(), "row": row})
        previous["repeats"].add(int(data["repeat"]))
        if previous["row"] != row:
            raise ValueError(f"non-deterministic comparison row: {key}")
    positions = sorted({position for position, _ in rows})
    modes = {mode for _, mode in rows}
    if modes != {"TreeMcts", "SharedTreeMcts"}:
        raise ValueError("comparison transcript must contain both MCTS modes")
    repeats = {len(entry["repeats"]) for entry in rows.values()}
    if len(repeats) != 1 or min(repeats) < 2:
        raise ValueError("comparison transcript needs at least two identical repeats")
    simulation_counts = {entry["row"]["simulations"] for entry in rows.values()}
    if len(simulation_counts) != 1:
        raise ValueError("comparison transcript has inconsistent simulation budgets")
    depth_counts = {entry["row"]["max_depth"] for entry in rows.values()}
    if len(depth_counts) != 1:
        raise ValueError("comparison transcript has inconsistent depth budgets")
    comparison = {
        "schema": "sekirei.mcts-comparison.v1",
        "simulations": simulation_counts.pop(),
        "max_depth": depth_counts.pop(),
        "repeats": repeats.pop(),
        "strength_claim": False,
        "positions": [],
    }
    for position in positions:
        comparison["positions"].append(
            {
                "name": position,
                "tree": {**rows[(position, "TreeMcts")]["row"]},
                "shared": {**rows[(position, "SharedTreeMcts")]["row"]},
            }
        )
    return comparison


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--release-manifest", type=Path, required=True)
    parser.add_argument("--transcript", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        release = json.loads(args.release_manifest.read_text())
        errors = validate(release)
        if errors:
            raise ValueError("invalid release manifest: " + ", ".join(errors))
        release["mcts_comparison"] = parse_comparison(args.transcript.read_text())
        errors = validate(release)
        if errors:
            raise ValueError("generated comparison is invalid: " + ", ".join(errors))
        args.output.write_text(json.dumps(release, indent=2) + "\n")
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        parser.error(str(exc))
    print(f"MCTS comparison manifest copy written: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
