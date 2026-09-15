#!/usr/bin/env python3
"""Compare fixed-depth core diagnostic runs without strength claims."""
from __future__ import annotations

import argparse
import json
from pathlib import Path


SCHEMA = "sekirei.floodgate-core-diagnostic.v2"


def load(path: Path) -> tuple[int, dict[tuple[str, int], dict], dict]:
    document = json.loads(path.read_text(encoding="utf-8"))
    if document.get("schema") != SCHEMA or document.get("diagnostic_only") is not True:
        raise ValueError(f"{path}: invalid diagnostic report")
    depth = document.get("execution", {}).get("options", {}).get("max_depth")
    if not isinstance(depth, int) or depth <= 0:
        raise ValueError(f"{path}: fixed max_depth is required")
    rows = {}
    for index, row in enumerate(document.get("results", [])):
        source = row.get("source", {})
        key = (str(source.get("game_id", "")), int(source.get("ply", row.get("index", index))))
        if not key[0] or key[1] < 0 or key in rows:
            raise ValueError(f"{path}: invalid or duplicate source key")
        measured = row.get("unrestricted", row)
        rows[key] = {
            "completion": measured.get("completion", "unknown"),
            "bestmove": measured.get("bestmove"),
            "score_cp": measured.get("score_cp"),
            "depth": measured.get("depth"),
            "bound": measured.get("bound", "unknown"),
            "aborted": measured.get("aborted"),
            "pv_usi": measured.get("pv_usi", []),
            "pv_legal": measured.get("pv_legal"),
            "pv_replay_preserves_input": measured.get("pv_replay_preserves_input"),
            "nodes": measured.get("nodes"),
            "elapsed_ms": measured.get("elapsed_ms"),
        }
    if not rows:
        raise ValueError(f"{path}: results must be non-empty")
    return depth, rows, document


def analyze(paths: list[Path]) -> dict:
    loaded = [load(path) for path in paths]
    depths = [item[0] for item in loaded]
    if depths != sorted(set(depths)):
        raise ValueError("depth runs must be supplied in ascending unique order")
    common = set(loaded[0][1])
    if any(set(rows) != common for _, rows, _ in loaded[1:]):
        raise ValueError("depth runs do not contain the same source positions")
    executions = [document.get("execution", {}) for _, _, document in loaded]
    baseline = executions[0]
    for execution in executions[1:]:
        if execution.get("weights") != baseline.get("weights"):
            raise ValueError("weights differ across depth runs")
        for field in ("threads", "spec_top_n", "use_book", "null_move_pruning"):
            if execution.get("options", {}).get(field) != baseline.get("options", {}).get(field):
                raise ValueError(f"execution option differs: {field}")
    positions = []
    for key in sorted(common):
        observations = [rows[key] for _, rows, _ in loaded]
        transitions = []
        for before, after, before_depth, after_depth in zip(observations, observations[1:], depths, depths[1:]):
            complete = all(
                item["completion"] == "search_completed"
                and item["aborted"] is False
                and item["bound"] == "exact"
                and item["pv_legal"] in (True, "true")
                and item["pv_replay_preserves_input"] in (True, "true")
                for item in (before, after)
            )
            transitions.append({
                "from_depth": before_depth,
                "to_depth": after_depth,
                "comparable": complete,
                "bestmove_changed": before["bestmove"] != after["bestmove"] if complete else None,
                "pv_changed": before["pv_usi"] != after["pv_usi"] if complete else None,
                "bound_changed": before["bound"] != after["bound"] if complete else None,
                "score_delta_cp": after["score_cp"] - before["score_cp"] if complete else None,
                "nodes_delta": after["nodes"] - before["nodes"] if complete else None,
            })
        positions.append({"game_id": key[0], "ply": key[1], "observations": [
            {**observation, "depth": depth, "actual_depth": observation["depth"]}
            for depth, observation in zip(depths, observations)
        ], "transitions": transitions})
    comparable = [t for p in positions for t in p["transitions"] if t["comparable"]]
    return {
        "schema": "sekirei.floodgate-depth-ladder-analysis.v1",
        "diagnostic_only": True,
        "depths": depths,
        "positions": positions,
        "summary": {
            "positions": len(positions),
            "transitions": len(comparable),
            "bestmove_changes": sum(t["bestmove_changed"] for t in comparable),
            "pv_changes": sum(t["pv_changed"] for t in comparable),
            "bound_changes": sum(t["bound_changed"] for t in comparable),
            "nonzero_score_deltas": sum(t["score_delta_cp"] != 0 for t in comparable),
        },
        "claims": {"strength": "not_permitted", "causal_inference": "not_proven",
                   "interpretation": "fixed-depth search observability only"},
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("runs", nargs="+", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if len(args.runs) < 2:
        parser.error("at least two depth runs are required")
    report = analyze(args.runs)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {report['summary']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
