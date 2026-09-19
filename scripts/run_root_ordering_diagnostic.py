#!/usr/bin/env python3
"""Record legal-generator order and completed root iterations for self-play cases.

The output is diagnostic-only.  It reports what this engine reached under a
fixed budget; it neither labels a correct move nor measures playing strength.
"""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import subprocess
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
FIXED_SPEC = importlib.util.spec_from_file_location("fixed", ROOT / "scripts" / "run_fixed_selfplay_diagnostic.py")
assert FIXED_SPEC and FIXED_SPEC.loader
FIXED = importlib.util.module_from_spec(FIXED_SPEC)
FIXED_SPEC.loader.exec_module(FIXED)
SWING_SPEC = importlib.util.spec_from_file_location("swing", ROOT / "scripts" / "run_selfplay_swing_diagnostic.py")
assert SWING_SPEC and SWING_SPEC.loader
SWING = importlib.util.module_from_spec(SWING_SPEC)
SWING_SPEC.loader.exec_module(SWING)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def parse_iteration_trace(raw: str, initial_order: list[str]) -> list[dict[str, Any]]:
    if len(set(initial_order)) != len(initial_order):
        raise ValueError("root initial order contains duplicate moves")
    rank = {move: index + 1 for index, move in enumerate(initial_order)}
    trace: list[dict[str, Any]] = []
    previous_depth = 0
    previous_nodes = 0
    if not raw:
        return trace
    for item in raw.split(","):
        fields = item.split(":")
        if len(fields) != 7:
            raise ValueError("invalid iteration trace entry")
        depth_text, bestmove, score_text, nodes_text, bound, mate_in_one_text, mate_blunder_text = fields
        try:
            depth, score, nodes, mate_in_one_nodes, mate_blunder_nodes = (
                int(depth_text), int(score_text), int(nodes_text), int(mate_in_one_text), int(mate_blunder_text)
            )
        except ValueError as exc:
            raise ValueError("iteration trace contains non-numeric depth, score, or nodes") from exc
        if depth <= previous_depth or nodes < previous_nodes:
            raise ValueError("iteration trace is not monotonic")
        if bound not in {"exact", "lower", "upper", "unknown"}:
            raise ValueError("iteration trace has invalid bound")
        if bestmove == "resign":
            if initial_order:
                raise ValueError("iteration trace resigned despite legal root moves")
            rank_value = None
        else:
            if bestmove not in rank:
                raise ValueError("iteration bestmove absent from legal-generator order")
            rank_value = rank[bestmove]
        trace.append({
            "depth": depth, "bestmove": bestmove, "score_cp": score, "nodes": nodes,
            "bound": bound, "initial_order_rank": rank_value,
            "root_mate_in_one_nodes": mate_in_one_nodes,
            "root_mate_blunder_nodes": mate_blunder_nodes,
        })
        previous_depth, previous_nodes = depth, nodes
    return trace


def run_case(engine: Path, weights: Path, item: dict[str, Any], nodes: int, disable_root_mate_safety: bool) -> dict[str, Any]:
    command = [
        str(engine), "--nodes", str(nodes), "--sfen", item["initial_sfen"],
        "--moves", " ".join(item["history_before_usi"]),
        "--expected-sfen", item["expected_sfen"], "--weights", str(weights),
        "--nnue-output", "absolute", "--iteration-trace",
    ]
    if disable_root_mate_safety:
        command.append("--disable-root-mate-safety")
    completed = subprocess.run(command, text=True, capture_output=True, check=False, timeout=180)
    if completed.returncode:
        raise RuntimeError(f"{item['id']}: {completed.stderr.strip()[-800:]}")
    fields = FIXED.parse_fields(completed.stdout)
    required = ("bestmove", "depth", "score_cp", "nodes", "completed_bound", "history_matches_expected",
                "history_replayed", "pv_legal", "pv_replay_preserves_input", "root_initial_order", "iteration_trace")
    if any(field not in fields for field in required):
        raise ValueError(f"{item['id']}: core output lacks root-ordering evidence")
    initial_order = [move for move in fields["root_initial_order"].split(",") if move]
    trace = parse_iteration_trace(fields["iteration_trace"], initial_order)
    if trace and trace[-1]["depth"] != int(fields["depth"]):
        raise ValueError(f"{item['id']}: final trace depth differs from reported depth")
    if trace and trace[-1]["bestmove"] != fields["bestmove"]:
        raise ValueError(f"{item['id']}: final trace bestmove differs from reported bestmove")
    return {
        "bestmove": fields["bestmove"], "score_cp": int(fields["score_cp"]), "depth": int(fields["depth"]),
        "nodes": int(fields["nodes"]), "elapsed_ms": int(fields.get("elapsed_ms", "0")),
        "completed_bound": fields["completed_bound"], "completed_iteration_valid": fields.get("completed_iteration_valid") == "true",
        "history_replayed": fields["history_replayed"] == "true",
        "history_matches_expected": fields["history_matches_expected"] == "true",
        "pv_legal": fields["pv_legal"] == "true",
        "pv_replay_preserves_input": fields["pv_replay_preserves_input"] == "true",
        "root_legal_move_count": len(initial_order), "root_initial_order": initial_order,
        "completed_iterations": trace,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--transcript", type=Path, required=True)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--weights", type=Path, required=True)
    parser.add_argument("--case", action="append", default=[])
    parser.add_argument("--nodes", type=int, required=True)
    parser.add_argument("--disable-root-mate-safety", action="store_true")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.nodes <= 0:
        parser.error("--nodes must be positive")
    try:
        games = SWING.read_transcript(args.transcript)
        selected = [SWING.case(games, game, seq, label) for game, seq, label in SWING.parse_selection(args.case)]
        results = [
            {**item, "trace": run_case(args.engine, args.weights, item, args.nodes, args.disable_root_mate_safety)}
            for item in selected
        ]
        document = {
            "schema": "sekirei.root-ordering-diagnostic.v1", "diagnostic_only": True,
            "strength_claim": False, "causal_inference": "not_proven",
            "contract": {"nodes": args.nodes, "threads": 1, "spec_top_n": 0, "nnue_output": "absolute",
                         "root_order": "legal-generator-order-before-search", "iterations": "completed-only",
                         "root_mate_safety": "disabled_diagnostic_only" if args.disable_root_mate_safety else "production"},
            "inputs": {"transcript": str(args.transcript), "transcript_sha256": sha256(args.transcript),
                       "engine": str(args.engine), "engine_sha256": sha256(args.engine),
                       "weights": str(args.weights), "weights_sha256": sha256(args.weights)},
            "results": results,
        }
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        print(json.dumps({"cases": len(results), "nodes": args.nodes}, sort_keys=True))
    except (OSError, ValueError, json.JSONDecodeError, RuntimeError) as exc:
        parser.error(str(exc))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
