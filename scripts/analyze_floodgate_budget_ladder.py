#!/usr/bin/env python3
"""Analyze fixed-node diagnostic runs as a budget ladder.

The report is diagnostic-only: it records depth, bound, PV, score, and
bestmove changes without treating a higher budget as a stronger label.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path


SCHEMA = "sekirei.floodgate-core-diagnostic.v2"


def load(path: Path) -> tuple[int, dict[tuple[str, int], dict], dict]:
    document = json.loads(path.read_text(encoding="utf-8"))
    if document.get("schema") != SCHEMA or document.get("diagnostic_only") is not True:
        raise ValueError(f"{path}: not a diagnostic-only core report")
    nodes = document.get("nodes")
    if not isinstance(nodes, int) or nodes <= 0:
        raise ValueError(f"{path}: nodes must be positive")
    rows: dict[tuple[str, int], dict] = {}
    for row_index, row in enumerate(document.get("results", [])):
        source = row.get("source", {})
        # Independent corpora have no game ply.  The runner's stable result
        # index is an acceptable positional key only when ply is absent.
        position_id = source.get("ply", row.get("index", row_index))
        key = (str(source.get("game_id", "")), int(position_id))
        if not key[0] or key[1] < 0 or key in rows:
            raise ValueError(f"{path}: invalid or duplicate source key")
        rows[key] = row
    if not rows:
        raise ValueError(f"{path}: results must be non-empty")
    return nodes, rows, document


def observation(row: dict) -> dict:
    # The corpus runner wraps the measured search as ``unrestricted``;
    # accepting a flat row keeps the analyzer usable with small fixtures.
    measured = row.get("unrestricted", row)
    return {
        "completion": measured.get("completion", "unknown"),
        "bestmove": measured.get("bestmove"),
        "score_cp": measured.get("score_cp"),
        "depth": measured.get("depth"),
        "bound": measured.get("bound", "unknown"),
        "aborted": measured.get("aborted"),
        "pv_usi": measured.get("pv_usi", []),
        "nodes": measured.get("nodes"),
        "elapsed_ms": measured.get("elapsed_ms"),
    }


def analyze(paths: list[Path]) -> dict:
    loaded = [load(path) for path in paths]
    budgets = [item[0] for item in loaded]
    if budgets != sorted(budgets) or len(set(budgets)) != len(budgets):
        raise ValueError("budget files must be supplied in ascending unique node order")
    common = set(loaded[0][1])
    if any(set(rows) != common for _, rows, _ in loaded[1:]):
        raise ValueError("budget runs do not contain the same source positions")
    executions = [document.get("execution", {}) for _, _, document in loaded]
    baseline_options = executions[0].get("options", {})
    for execution in executions[1:]:
        options = execution.get("options", {})
        for field in ("threads", "spec_top_n", "use_book", "null_move_pruning", "max_depth"):
            if options.get(field) != baseline_options.get(field):
                raise ValueError(f"execution option differs: {field}")
        if execution.get("weights") != executions[0].get("weights"):
            raise ValueError("weights differ across budget runs")
    positions = []
    for key in sorted(common):
        observations = [observation(rows[key]) for _, rows, _ in loaded]
        transitions = []
        for before, after, before_budget, after_budget in zip(
            observations, observations[1:], budgets, budgets[1:]
        ):
            complete = all(
                item["completion"] == "search_completed"
                and item["aborted"] is False
                and item["bound"] == "exact"
                for item in (before, after)
            )
            transitions.append({
                "from_nodes": before_budget,
                "to_nodes": after_budget,
                "comparable": complete,
                "bestmove_changed": before["bestmove"] != after["bestmove"] if complete else None,
                "score_delta_cp": (
                    after["score_cp"] - before["score_cp"]
                    if complete and isinstance(before["score_cp"], int)
                    and isinstance(after["score_cp"], int) else None
                ),
                "depth_delta": (
                    after["depth"] - before["depth"]
                    if complete and isinstance(before["depth"], int)
                    and isinstance(after["depth"], int) else None
                ),
                "pv_changed": before["pv_usi"] != after["pv_usi"] if complete else None,
                "bound_changed": before["bound"] != after["bound"] if complete else None,
            })
        positions.append({
            "game_id": key[0],
            "ply": key[1],
            "observations": [
                {"nodes": budget, **obs} for budget, obs in zip(budgets, observations)
            ],
            "transitions": transitions,
        })
    comparable = [transition for position in positions for transition in position["transitions"] if transition["comparable"]]
    return {
        "schema": "sekirei.floodgate-budget-ladder-analysis.v1",
        "diagnostic_only": True,
        "budgets_nodes": budgets,
        "positions": positions,
        "summary": {
            "positions": len(positions),
            "transitions": len(comparable),
            "bestmove_changes": sum(item["bestmove_changed"] is True for item in comparable),
            "pv_changes": sum(item["pv_changed"] is True for item in comparable),
            "bound_changes": sum(item["bound_changed"] is True for item in comparable),
            "nonzero_score_deltas": sum(item["score_delta_cp"] not in (None, 0) for item in comparable),
            "depth_increases": sum((item["depth_delta"] or 0) > 0 for item in comparable),
        },
        "claims": {
            "strength": "not_permitted",
            "causal_inference": "not_proven",
            "interpretation": "budget sensitivity and search observability only",
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("runs", nargs="+", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if len(args.runs) < 2:
        parser.error("at least two budget runs are required")
    report = analyze(args.runs)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {report['summary']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
