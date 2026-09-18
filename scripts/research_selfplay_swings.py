#!/usr/bin/env python3
"""Re-search frozen self-play score swings under one-factor fixed-node cells.

This preserves the original initial SFEN and complete move history.  It is a
diagnostic only: a score change, a pruning difference, or a recovered move is
not an Elo result and cannot select a release candidate by itself.
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


CELLS = {
    "cold_baseline": {},
    "warm_baseline": {"warmup_nodes": 20_000},
    "cold_nmp_off": {"disable_nmp": True},
    "cold_lmr_off": {"disable_lmr": True},
    "cold_nmp_lmr_off": {"disable_nmp": True, "disable_lmr": True},
}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def run_one(engine: Path, row: dict[str, Any], weights: Path, nodes: int, root_move: str | None, cell: dict[str, Any]) -> dict[str, Any]:
    command = [str(engine), "--nodes", str(nodes), "--sfen", row["initial_sfen"], "--moves", " ".join(row["history"]), "--expected-sfen", row["sfen"], "--weights", str(weights), "--nnue-output", "absolute"]
    if root_move:
        command.extend(["--root-move", root_move])
    if cell.get("warmup_nodes"):
        command.extend(["--warmup-nodes", str(cell["warmup_nodes"])])
    if cell.get("disable_nmp"):
        command.append("--disable-nmp")
    if cell.get("disable_lmr"):
        command.append("--disable-lmr")
    completed = subprocess.run(command, text=True, capture_output=True, check=False, timeout=180)
    if completed.returncode:
        raise RuntimeError(f"diagnostic failed ({completed.returncode}): {completed.stderr.strip()[-1000:]}")
    fields = FIXED.parse_fields(completed.stdout)
    required = {"history_replayed", "history_matches_expected", "history_moves", "bound", "completed_bound", "aborted"}
    missing = required - fields.keys()
    if missing:
        raise RuntimeError(f"diagnostic missing {sorted(missing)}")
    return {
        "bestmove": fields["bestmove"], "depth": int(fields["depth"]), "score_cp": int(fields["score_cp"]),
        "nodes": int(fields["nodes"]), "elapsed_ms": int(fields["elapsed_ms"]),
        "bound": fields["bound"], "completed_bound": fields["completed_bound"],
        "completed_iteration_valid": fields["completed_iteration_valid"] == "true", "aborted": fields["aborted"] == "true",
        "pv_usi": fields["pv_usi"].split(",") if fields["pv_usi"] else [], "pv_legal": fields["pv_legal"] == "true",
        "history_replayed": fields["history_replayed"] == "true", "history_matches_expected": fields["history_matches_expected"] == "true",
        "history_moves": int(fields["history_moves"]),
    }


def unique_rows(analysis: dict, limit: int) -> list[dict]:
    selected, seen = [], set()
    for row in analysis.get("top_swings", []):
        key = (row.get("sfen"), tuple(row.get("history", [])))
        if key in seen:
            continue
        if not isinstance(row.get("sfen"), str) or not isinstance(row.get("initial_sfen"), str):
            continue
        if not isinstance(row.get("history"), list) or not all(isinstance(move, str) for move in row["history"]):
            continue
        seen.add(key)
        selected.append(row)
        if len(selected) == limit:
            break
    if len(selected) != limit:
        raise ValueError(f"need {limit} unique valid swing rows, found {len(selected)}")
    return selected


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--analysis", type=Path, required=True)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--weights", type=Path, required=True)
    parser.add_argument("--nodes", type=int, default=50_000)
    parser.add_argument("--limit", type=int, default=5)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.nodes <= 0 or args.limit <= 0:
        parser.error("--nodes and --limit must be positive")
    analysis = json.loads(args.analysis.read_text(encoding="utf-8"))
    rows = unique_rows(analysis, args.limit)
    results = []
    for index, row in enumerate(rows, 1):
        actual_move = row.get("actual_move")
        cells = {
            name: {
                "free": run_one(args.engine, row, args.weights, args.nodes, None, config),
                "actual_root": run_one(args.engine, row, args.weights, args.nodes, actual_move, config),
            }
            for name, config in CELLS.items()
        }
        results.append({
            "id": f"swing-{index:02d}",
            "source": {key: row[key] for key in ("game", "seq", "previous_cp", "cp", "delta", "depth", "previous_depth", "in_check", "sfen", "actual_move", "initial_sfen", "history")},
            "results": cells,
        })
    document = {
        "schema": "sekirei.selfplay-swing-research.v1", "diagnostic_only": True, "strength_claim": False,
        "causal_inference": "not_proven", "contract": {"nodes": args.nodes, "threads": 1, "spec_top_n": 0, "history": "initial_sfen plus full recorded USI history", "cells": CELLS, "modes": ["free", "actual_root"], "one_factor_rule": "each non-baseline cell changes only its named pruning or warm-TT state"},
        "inputs": {"analysis": str(args.analysis), "analysis_sha256": sha256(args.analysis), "engine": str(args.engine), "engine_sha256": sha256(args.engine), "weights": str(args.weights), "weights_sha256": sha256(args.weights)},
        "results": results,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"positions": len(results), "cells_per_mode": len(CELLS), "searches": len(results) * len(CELLS) * 2}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
