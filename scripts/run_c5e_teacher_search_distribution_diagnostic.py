#!/usr/bin/env python3
"""Separate fixed-NNUE static output, search labels, and position mix for C5e.

Material search is an anchor for diagnosing scale compression, not a ground
truth label.  Mate-like search scores are excluded from centipawn statistics.
No result produced here is an Elo or candidate-adoption result.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import subprocess
from collections import Counter
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("fixed", ROOT / "run_fixed_selfplay_diagnostic.py")
assert SPEC and SPEC.loader
FIXED = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(FIXED)

MATE_ABS_MIN = 899_000


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def read_jsonl(path: Path) -> list[dict[str, Any]]:
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]


def choose(rows: list[dict[str, Any]], limit: int) -> list[dict[str, Any]]:
    rows.sort(key=lambda row: hashlib.sha256((row["source"]["path"] + "\0" + row["sfen"]).encode()).hexdigest())
    return rows[:limit]


def static_scores(probe: Path, weights: Path, rows: list[dict[str, Any]]) -> dict[str, int]:
    command = [str(probe), str(weights), "--json", "--nnue-output", "absolute"]
    for row in rows:
        command.extend(["--sfen", row["sfen"]])
    completed = subprocess.run(command, text=True, capture_output=True, check=False, timeout=120)
    if completed.returncode:
        raise RuntimeError(completed.stderr[-1000:])
    document = json.loads(completed.stdout)
    scores = {item["sfen"]: item["score_cp"] for item in document.get("probes", [])}
    if len(scores) != len(rows) or any(row["sfen"] not in scores for row in rows):
        raise ValueError("static NNUE probe did not return every selected SFEN")
    return scores


def complete(result: dict[str, Any]) -> bool:
    return result.get("completed_iteration_valid") is True and result.get("completed_bound") == "exact" and result.get("pv_legal") is True


def mix(rows: list[dict[str, Any]]) -> dict[str, Any]:
    return {
        "positions": len(rows),
        "phase": dict(sorted(Counter(row.get("tags", {}).get("phase", "missing") for row in rows).items())),
        "side_to_move": dict(sorted(Counter(row.get("tags", {}).get("side_to_move", "missing") for row in rows).items())),
        "in_check": dict(sorted(Counter(str(row.get("tags", {}).get("in_check", False)).lower() for row in rows).items())),
        "has_capture": dict(sorted(Counter(str(row.get("tags", {}).get("has_capture", False)).lower() for row in rows).items())),
        "games": len({row.get("source", {}).get("game_number") for row in rows}),
        "opening_groups": len({row.get("source", {}).get("opening") for row in rows}),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--split-manifest", type=Path, required=True)
    parser.add_argument("--probe", type=Path, required=True)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--weights", type=Path, required=True)
    parser.add_argument("--nodes", type=int, default=100_000)
    parser.add_argument("--limit", type=int, default=128)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.nodes <= 0 or args.limit <= 0:
        parser.error("nodes and limit must be positive")
    split = json.loads(args.split_manifest.read_text(encoding="utf-8"))
    if split.get("schema") != "sekirei.selfplay-calibration-split.v1" or split.get("status") != "frozen":
        parser.error("split manifest must be frozen")
    train_path, holdout_path = Path(split["train"]["path"]), Path(split["holdout"]["path"])
    train, holdout = read_jsonl(train_path), read_jsonl(holdout_path)
    selected = choose(holdout, args.limit)
    static = static_scores(args.probe, args.weights, selected)
    results = []
    for index, row in enumerate(selected, 1):
        input_row = {"pre_move_sfen": row["sfen"]}
        material = FIXED.run_one(args.engine, input_row, nodes=args.nodes, weights=None, root_move=None)
        nnue_search = FIXED.run_one(args.engine, input_row, nodes=args.nodes, weights=args.weights, root_move=None)
        results.append({"id": f"c5e-{index:03d}", "source": row["source"], "tags": row["tags"], "static_nnue_cp": static[row["sfen"]], "material_search": material, "fixed_nnue_search": nnue_search})
    completed = [row for row in results if complete(row["material_search"]) and complete(row["fixed_nnue_search"])]
    cp_rows = [row for row in completed if abs(row["material_search"]["score_cp"]) < MATE_ABS_MIN and abs(row["fixed_nnue_search"]["score_cp"]) < MATE_ABS_MIN]
    anchors = [row for row in cp_rows if abs(row["material_search"]["score_cp"]) >= 1_000]
    static_compressed = [row for row in anchors if abs(row["static_nnue_cp"]) <= 100]
    search_compressed = [row for row in anchors if abs(row["fixed_nnue_search"]["score_cp"]) <= 100]
    both_compressed = [row for row in anchors if row in static_compressed and row in search_compressed]
    document = {
        "schema": "sekirei.c5e-teacher-search-distribution-diagnostic.v1", "diagnostic_only": True, "strength_claim": False,
        "contract": {"nodes": args.nodes, "threads": 1, "spec_top_n": 0, "tt": "cold_process_per_search", "nnue_output": "absolute", "selection": "SHA-256(source.path + NUL + SFEN), first limit", "mate_abs_min": MATE_ABS_MIN},
        "inputs": {"split_manifest": str(args.split_manifest), "split_manifest_sha256": sha256(args.split_manifest), "probe": str(args.probe), "probe_sha256": sha256(args.probe), "engine": str(args.engine), "engine_sha256": sha256(args.engine), "weights": str(args.weights), "weights_sha256": sha256(args.weights)},
        "distribution": {"train": mix(train), "holdout": mix(holdout), "selected_holdout": mix(selected), "position_split": split.get("split_kind"), "limitation": split.get("limitation")},
        "results": results,
        "summary": {"selected": len(results), "completed": len(completed), "cp_comparable": len(cp_rows), "mate_like_excluded": len(completed) - len(cp_rows), "material_anchors_abs_ge_1000": len(anchors), "static_nnue_compressed_abs_le_100": len(static_compressed), "fixed_nnue_search_compressed_abs_le_100": len(search_compressed), "both_static_and_search_compressed": len(both_compressed), "inference": "static-dominant" if len(anchors) and len(both_compressed) == len(anchors) else "search-or-mixed", "interpretation": "Static-dominant means the observed compression is already present before search on this diagnostic sample. It does not prove that material is correct or identify a safe training target."},
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(document["summary"], sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
