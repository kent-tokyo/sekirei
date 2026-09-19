#!/usr/bin/env python3
"""Re-search selected self-play swings with preserved game-local history.

This creates diagnostic evidence only.  A difference across cells can be a
useful reproduction condition, but is neither move regret nor a strength
measurement.  Every cell begins a new process; `warm_tt` is the sole
intentional within-process warmup.
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
SPEC = importlib.util.spec_from_file_location("fixed", ROOT / "scripts" / "run_fixed_selfplay_diagnostic.py")
assert SPEC and SPEC.loader
FIXED = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(FIXED)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def source_evaluator(transcript: Path) -> dict[str, Any]:
    """Describe the run that supplied positions without relabelling its scores.

    Old manifests did not have an explicit evaluator object.  A missing weight
    is therefore preserved as material/unknown source evidence, while the
    re-search below always records its own explicit NNUE weight separately.
    """
    manifest_path = transcript.parent / "run-manifest.json"
    if not manifest_path.is_file():
        return {"manifest": None, "kind": "unknown", "observed_scores_usable": False}
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise ValueError(f"invalid source run manifest: {manifest_path}") from exc
    evaluator = manifest.get("evaluator")
    if isinstance(evaluator, dict):
        kind = evaluator.get("kind")
        if kind in {"nnue", "material"}:
            return {
                "manifest": str(manifest_path),
                "manifest_sha256": sha256(manifest_path),
                "kind": kind,
                "weights_sha256": manifest.get("weights", {}).get("sha256")
                if isinstance(manifest.get("weights"), dict) else None,
                "observed_scores_usable": kind == "nnue",
            }
    weights = manifest.get("weights")
    weight_hash = weights.get("sha256") if isinstance(weights, dict) else None
    return {
        "manifest": str(manifest_path),
        "manifest_sha256": sha256(manifest_path),
        "kind": "nnue" if weight_hash else "material_or_unknown_legacy",
        "weights_sha256": weight_hash,
        "observed_scores_usable": bool(weight_hash),
    }


def read_transcript(path: Path) -> dict[int, list[dict[str, Any]]]:
    games: dict[int, list[dict[str, Any]]] = {}
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip():
            continue
        row = json.loads(line)
        if row.get("verdict") != "ok":
            continue
        game, seq = row.get("game_num"), row.get("seq")
        if not isinstance(game, int) or not isinstance(seq, int) or not isinstance(row.get("sfen_before"), str):
            raise ValueError(f"transcript line {line_number} lacks game/seq/SFEN")
        search = row.get("search")
        if not isinstance(search, dict) or not isinstance(row.get("raw_bestmove"), str):
            raise ValueError(f"transcript line {line_number} lacks recorded move")
        games.setdefault(game, []).append(row)
    for game, rows in games.items():
        rows.sort(key=lambda row: row["seq"])
        expected = list(range(rows[0]["seq"], rows[0]["seq"] + len(rows)))
        if [row["seq"] for row in rows] != expected:
            raise ValueError(f"game {game}: transcript sequence is not contiguous")
    return games


def case(games: dict[int, list[dict[str, Any]]], game: int, seq: int, label: str) -> dict[str, Any]:
    rows = games.get(game, [])
    matches = [index for index, row in enumerate(rows) if row["seq"] == seq]
    if len(matches) != 1:
        raise ValueError(f"game {game}: no unique seq {seq}")
    index = matches[0]
    target = rows[index]
    history = [row["raw_bestmove"] for row in rows[:index]]
    return {
        "id": label,
        "game_num": game,
        "seq": seq,
        "initial_sfen": rows[0]["sfen_before"],
        "history_before_usi": history,
        "expected_sfen": target["sfen_before"],
        "observed": {
            "bestmove": target["raw_bestmove"],
            "score_cp": target["search"].get("score_cp"),
            "score_mate": target["search"].get("score_mate"),
            "depth": target["search"].get("depth"),
            "nodes": target["search"].get("nodes"),
        },
    }


def parse_selection(values: list[str]) -> list[tuple[int, int, str]]:
    selected = []
    for value in values:
        try:
            game_text, seq_text, label = value.split(":", 2)
            selected.append((int(game_text), int(seq_text), label))
        except ValueError as exc:
            raise ValueError(f"--case must be GAME:SEQ:LABEL, got {value!r}") from exc
    if not selected:
        raise ValueError("at least one --case is required")
    if len({label for _, _, label in selected}) != len(selected):
        raise ValueError("case labels must be unique")
    return selected


def search(
    engine: Path,
    weights: Path | None,
    item: dict[str, Any],
    *,
    nodes: int,
    cell: str,
    depth: int,
    nnue_output: str,
) -> dict[str, Any]:
    command = [
        str(engine), "--nodes", str(nodes), "--sfen", item["initial_sfen"],
        "--moves", " ".join(item["history_before_usi"]),
        "--expected-sfen", item["expected_sfen"],
    ]
    if weights is not None:
        command.extend(["--weights", str(weights), "--nnue-output", nnue_output])
    if cell == "actual_root":
        command.extend(["--root-move", item["observed"]["bestmove"]])
    elif cell == "warm_tt":
        command.extend(["--warmup-nodes", str(max(1, nodes // 5))])
    elif cell == "nmp_off":
        command.append("--disable-nmp")
    elif cell == "lmr_off":
        command.append("--disable-lmr")
    elif cell == "fixed_depth":
        command.extend(["--max-depth", str(depth)])
    elif cell != "free":
        raise ValueError(f"unknown cell {cell}")
    completed = subprocess.run(command, text=True, capture_output=True, check=False, timeout=180)
    if completed.returncode:
        raise RuntimeError(f"{item['id']}/{cell}: {completed.stderr.strip()[-800:]}")
    fields = FIXED.parse_fields(completed.stdout)
    required = ("history_matches_expected", "history_replayed", "pv_replay_preserves_input", "completed_bound")
    if any(key not in fields for key in required):
        raise ValueError(f"{item['id']}/{cell}: missing history or completion field")
    return {
        "bestmove": fields["bestmove"], "score_cp": int(fields["score_cp"]), "depth": int(fields["depth"]),
        "nodes": int(fields["nodes"]), "elapsed_ms": int(fields["elapsed_ms"]),
        "bound": fields.get("bound"), "completed_bound": fields["completed_bound"],
        "completed_iteration_valid": fields["completed_iteration_valid"] == "true",
        "pv_legal": fields["pv_legal"] == "true",
        "pv_replay_preserves_input": fields["pv_replay_preserves_input"] == "true",
        "history_replayed": fields["history_replayed"] == "true",
        "history_matches_expected": fields["history_matches_expected"] == "true",
        "history_moves": int(fields.get("history_moves", "0")),
        "warmup": {key.removeprefix("warmup_"): value for key, value in fields.items() if key.startswith("warmup_")},
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--transcript", type=Path, required=True)
    parser.add_argument("--engine", type=Path, required=True)
    evaluator = parser.add_mutually_exclusive_group(required=True)
    evaluator.add_argument("--weights", type=Path)
    evaluator.add_argument("--material-only", action="store_true")
    parser.add_argument(
        "--nnue-output",
        choices=("absolute", "residual-material"),
        default="absolute",
        help="explicit evaluator interpretation for every NNUE re-search cell",
    )
    parser.add_argument("--case", action="append", default=[])
    parser.add_argument("--nodes", type=int, default=50_000)
    parser.add_argument("--fixed-depth", type=int, default=4)
    parser.add_argument(
        "--cells",
        default="free,actual_root,warm_tt,nmp_off,lmr_off,fixed_depth",
        help="comma-separated subset of free,actual_root,warm_tt,nmp_off,lmr_off,fixed_depth",
    )
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.nodes <= 0 or args.fixed_depth <= 0:
        parser.error("--nodes and --fixed-depth must be positive")
    if args.material_only and args.nnue_output != "absolute":
        parser.error("--nnue-output is meaningful only with --weights")
    try:
        games = read_transcript(args.transcript)
        selected = [case(games, game, seq, label) for game, seq, label in parse_selection(args.case)]
        allowed_cells = ("free", "actual_root", "warm_tt", "nmp_off", "lmr_off", "fixed_depth")
        cells = tuple(cell for cell in args.cells.split(",") if cell)
        if not cells or len(set(cells)) != len(cells) or any(cell not in allowed_cells for cell in cells):
            raise ValueError("--cells must be a non-empty unique subset of " + ",".join(allowed_cells))
        results = []
        for item in selected:
            cells_result = {
                cell: search(
                    args.engine,
                    args.weights,
                    item,
                    nodes=args.nodes,
                    cell=cell,
                    depth=args.fixed_depth,
                    nnue_output=args.nnue_output,
                )
                for cell in cells
            }
            results.append({**item, "cells": cells_result})
        document = {
            "schema": "sekirei.selfplay-swing-diagnostic.v1", "diagnostic_only": True,
            "strength_claim": False, "causal_inference": "not_proven",
            "contract": {"nodes": args.nodes, "fixed_depth": args.fixed_depth, "threads": 1, "spec_top_n": 0,
                         "evaluator": "nnue" if args.weights else "material",
                         "nnue_output": args.nnue_output if args.weights else None,
                         "cells": list(cells), "tt": "cold except warm_tt"},
            "inputs": {"transcript": str(args.transcript), "transcript_sha256": sha256(args.transcript),
                       "source_evaluator": source_evaluator(args.transcript),
                       "engine": str(args.engine), "engine_sha256": sha256(args.engine),
                       "evaluator": "nnue" if args.weights else "material",
                       "weights": str(args.weights) if args.weights else None,
                       "weights_sha256": sha256(args.weights) if args.weights else None},
            "results": results,
        }
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        print(json.dumps({"cases": len(results), "cells_per_case": len(cells)}, sort_keys=True))
    except (OSError, ValueError, json.JSONDecodeError, RuntimeError) as exc:
        parser.error(str(exc))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
