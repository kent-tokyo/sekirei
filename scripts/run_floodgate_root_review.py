#!/usr/bin/env python3
"""Create a bounded, diagnostic-only root review from Floodgate records.

The runner pairs loss sign reversals with a small number of winning controls.
It records full-history replay, cold/warm TT behavior, and actual-root scores,
but never treats the played move as a correct-move label or a strength result.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
from pathlib import Path

from analyze_analysis_record import analyze
from run_core_floodgate_diagnostic import csa_move_to_usi, run_history_replay


def file_sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def select_positions(csa_dir: Path, analysis_dir: Path, controls_per_win: int = 2) -> list[dict]:
    selected = []
    for csa in sorted(csa_dir.glob("*.csa")):
        analysis = analysis_dir / f"{csa.stem}.analysis.jsonl"
        if not analysis.is_file():
            continue
        report = analyze(csa, analysis)
        candidates = [
            row for row in report["records"]
            if row.get("sfen") and row.get("actual_move_csa") and not row.get("terminal")
            and (
                (report["result"] == "lose" and row.get("sign_reversal"))
                or (
                    report["result"] == "win"
                    and isinstance(row.get("score_delta_cp"), int)
                    and row["score_delta_cp"] > 0
                )
            )
        ]
        if report["result"] == "win":
            candidates = candidates[:controls_per_win]
        for row in candidates:
            selected.append({
                "game_id": report["game_id"],
                "result": report["result"],
                "csa": csa,
                "analysis": analysis,
                "ply": row["ply"],
                "sfen": row["sfen"],
                "played_move_csa": row["actual_move_csa"],
                "source_score_delta_cp": row.get("score_delta_cp"),
            })
    return selected


def parse_search(stdout: str) -> dict:
    values = {}
    numeric = {"depth", "score_cp", "nodes", "elapsed_ms", "warmup_depth", "warmup_score_cp", "warmup_nodes"}
    for field in stdout.strip().split("\t"):
        key, separator, value = field.partition("=")
        if separator:
            values[key] = int(value) if key in numeric else value
    return values


def search(binary: Path, sfen: str, root_move: str | None, warmup: bool, nodes: int, depth: int,
           disable_nmp: bool = False) -> dict:
    command = [str(binary), "--nodes", str(nodes), "--max-depth", str(depth), "--sfen", sfen]
    if disable_nmp:
        command.append("--disable-nmp")
    if warmup:
        command.extend(("--warmup-nodes", str(nodes)))
    if root_move:
        command.extend(("--root-move", root_move))
    completed = subprocess.run(command, capture_output=True, text=True, timeout=30, check=False)
    if completed.returncode != 0:
        return {"completion": "process_error", "returncode": completed.returncode}
    result = parse_search(completed.stdout)
    result["completion"] = "search_completed"
    return result


def review(binary: Path, history_binary: Path, csa_dir: Path, analysis_dir: Path,
           nodes: int, depth: int, controls_per_win: int, disable_nmp: bool = False) -> dict:
    rows = []
    for position in select_positions(csa_dir, analysis_dir, controls_per_win):
        played = csa_move_to_usi(position["played_move_csa"], position["sfen"])
        cold = search(binary, position["sfen"], None, False, nodes, depth, disable_nmp)
        warm = search(binary, position["sfen"], None, True, nodes, depth, disable_nmp)
        actual = search(binary, position["sfen"], played, False, nodes, depth, disable_nmp)
        replay = run_history_replay(history_binary, position["csa"], position["sfen"], position["ply"], 30.0)
        rows.append({
            "game_id": position["game_id"],
            "result": position["result"],
            "ply": position["ply"],
            "played_move_usi": played,
            "played_move_is_label": False,
            "source_score_delta_cp": position["source_score_delta_cp"],
            "source": {
                "csa": str(position["csa"]),
                "csa_sha256": file_sha256(position["csa"]),
                "analysis": str(position["analysis"]),
                "analysis_sha256": file_sha256(position["analysis"]),
            },
            "history_replay": replay,
            "cold": cold,
            "warm": warm,
            "actual_root": actual,
            "comparison": {
                "requested_node_budget": nodes,
                "same_requested_budget": True,
                "bestmove_matches_played": cold.get("bestmove") == played,
                "warm_bestmove_changed": cold.get("bestmove") != warm.get("bestmove"),
                "warm_score_delta_cp": warm.get("score_cp", 0) - cold.get("score_cp", 0),
                "actual_score_delta_cp": actual.get("score_cp", 0) - cold.get("score_cp", 0),
            },
        })
    return {
        "schema": "sekirei.floodgate-root-review.v2",
        "diagnostic_only": True,
        "contract": {
            "nodes": nodes,
            "max_depth": depth,
            "threads": 1,
            "spec_top_n": 0,
            "disable_nmp": disable_nmp,
            "played_move_is_label": False,
        },
        "binary": {"path": str(binary), "sha256": file_sha256(binary)},
        "history_binary": {"path": str(history_binary), "sha256": file_sha256(history_binary)},
        "rows": rows,
        "strength_claim": "not_permitted",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--history-binary", type=Path, required=True)
    parser.add_argument("--csa-dir", type=Path, required=True)
    parser.add_argument("--analysis-dir", type=Path, required=True)
    parser.add_argument("--nodes", type=int, default=20_000)
    parser.add_argument("--max-depth", type=int, default=2)
    parser.add_argument("--controls-per-win", type=int, default=2)
    parser.add_argument("--disable-nmp", action="store_true")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.nodes <= 0 or args.max_depth <= 0 or args.controls_per_win <= 0:
        parser.error("nodes, max-depth, and controls-per-win must be positive")
    document = review(args.binary, args.history_binary, args.csa_dir, args.analysis_dir,
                      args.nodes, args.max_depth, args.controls_per_win, args.disable_nmp)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {len(document['rows'])} rows")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
