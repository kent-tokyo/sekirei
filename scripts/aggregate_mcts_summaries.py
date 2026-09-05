#!/usr/bin/env python3
"""Aggregate validated MCTS comparison summaries across fixed budgets."""
import argparse
import json
from pathlib import Path

from summarize_mcts_comparison import summarize


def aggregate(paths: list[Path]) -> dict:
    if not paths:
        raise ValueError("at least one comparison manifest is required")
    summaries = [summarize(path) for path in paths]
    releases = {summary["release"] for summary in summaries}
    if len(releases) != 1:
        raise ValueError("comparison manifests must use the same release")
    budgets = sorted(
        (
            {
                "simulations": summary["simulations"],
                "max_depth": summary["max_depth"],
                "repeats": summary["repeats"],
                "positions": summary["positions"],
            }
            for summary in summaries
        ),
        key=lambda budget: (budget["simulations"], budget["max_depth"]),
    )
    keys = {(budget["simulations"], budget["max_depth"]) for budget in budgets}
    if len(keys) != len(budgets):
        raise ValueError("duplicate MCTS budgets are not allowed")
    return {
        "schema": "sekirei.mcts-budget-summary.v1",
        "release": releases.pop(),
        "strength_claim": False,
        "budgets": budgets,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("manifests", type=Path, nargs="+")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    try:
        rendered = json.dumps(aggregate(args.manifests), indent=2) + "\n"
        if args.output:
            args.output.write_text(rendered)
            print(f"MCTS budget summary written: {args.output}")
        else:
            print(rendered, end="")
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        parser.error(str(exc))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
