#!/usr/bin/env python3
"""Freeze Q21o's fixed-depth versus fixed-node teacher-contract audit."""

from __future__ import annotations

import argparse
import hashlib
import json
import statistics
from pathlib import Path
from typing import Any


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def bind(path: Path) -> dict[str, str]:
    return {"path": str(path), "sha256": sha256(path)}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def stable_rank(identifier: str, seed: int) -> str:
    return hashlib.sha256(f"{seed}\0{identifier}".encode()).hexdigest()


def top_moves(row: dict[str, Any], limit: int) -> list[str]:
    candidates = row.get("teacher_root", {}).get("root_candidates", [])
    ordinary = [
        (candidate["move"], candidate["score_cp"])
        for candidate in candidates
        if isinstance(candidate.get("move"), str)
        and isinstance(candidate.get("score_cp"), int)
        and abs(candidate["score_cp"]) <= 10_000
    ]
    require(ordinary, f"{row.get('id')}: no ordinary depth-3 root move")
    return [move for move, _ in sorted(ordinary, key=lambda item: (-item[1], item[0]))[:limit]]


def prepare(args: argparse.Namespace) -> dict[str, Any]:
    q21n = json.loads(args.q21n_decision.read_text(encoding="utf-8"))
    summary = json.loads(args.q21n_summary.read_text(encoding="utf-8"))
    shallow = json.loads(args.shallow.read_text(encoding="utf-8"))
    corpus = json.loads(args.corpus.read_text(encoding="utf-8"))
    depth7 = json.loads(args.depth7_measurements.read_text(encoding="utf-8"))
    require(
        q21n.get("schema") == "sekirei.q21n-final-decision.v1"
        and q21n.get("status") == "complete"
        and q21n.get("classification") == "teacher_depth_insufficient",
        "Q21o requires Q21n's completed teacher-depth-insufficient decision",
    )
    require(summary.get("schema") == "sekirei.q21n-teacher-depth-audit.v1", "unexpected Q21n summary")
    shallow_rows = {row["id"]: row for row in shallow.get("rows", [])}
    positions = {row["id"]: row for row in corpus.get("positions", [])}
    problem_ids = [row["position_id"] for row in q21n["rows"]]
    require(len(problem_ids) == 3 and len(set(problem_ids)) == 3, "Q21o requires all three Q21n adjudication parents")

    controls = []
    for phase in ("opening", "middlegame", "endgame"):
        bucket = [
            row for row in summary.get("rows", [])
            if row.get("phase") == phase
            and row.get("ordinary_cp_comparable") is True
            and row.get("deep_regret_cp", 999_999) < 100
            and row.get("position_id") not in problem_ids
        ]
        require(bucket, f"no stable {phase} control")
        controls.append(min(bucket, key=lambda row: stable_rank(row["position_id"], args.control_seed)))

    selected = []
    summary_rows = {row["position_id"]: row for row in summary["rows"]}
    for role, identifiers in (("problem", problem_ids), ("control", [row["position_id"] for row in controls])):
        for identifier in identifiers:
            require(identifier in positions and identifier in shallow_rows, f"missing selected parent {identifier}")
            row = summary_rows[identifier]
            selected.append({
                "position_id": identifier,
                "role": role,
                "phase": row["phase"],
                "material_band": row["material_band"],
                "forcing_class": row["forcing_class"],
                "depth5_regret_cp": row["deep_regret_cp"],
                "depth3_top_moves": row["shallow_top_moves"],
                "depth3_top8_moves": top_moves(shallow_rows[identifier], 8),
            })
    depth7_nodes = [row["free"]["nodes"] for row in depth7.get("rows", [])]
    require(len(depth7_nodes) == 3 and all(isinstance(nodes, int) and nodes > 0 for nodes in depth7_nodes), "Q21n depth-7 node evidence missing")
    node_budget = int(round(statistics.median(depth7_nodes), -5))
    require(node_budget == 3_200_000, "Q21o fixed-node budget derivation changed")
    return {
        "schema": "sekirei.q21o-teacher-contract-preregistration.v1",
        "status": "frozen_before_measurement",
        "diagnostic_only": True,
        "strength_claim": False,
        "question": "Which stronger fixed-T search contract is deterministic, bounded, and cross-contract robust for root labels?",
        "selection": {
            "problem_rule": "all three Q21n depth-7 adjudication parents",
            "control_rule": "one depth5-regret<100cp parent per phase by stable hash before Q21o scores",
            "control_seed": args.control_seed,
            "parents": selected,
        },
        "arms": {
            "depth7": {"max_depth": 7, "nodes": None},
            "nodes3200k": {"max_depth": None, "nodes": node_budget},
        },
        "measurement_contract": {
            "threads": 1,
            "spec_top_n": 0,
            "cold_process_per_search": True,
            "calibration_repeats": 2,
            "calibration_searches": "one free plus every tied depth-3 top move",
            "ranking_candidates": "depth-3 top 8 union every calibration free bestmove from both arms",
            "ranking_repeats": 1,
            "timeout_seconds_per_search": 600,
            "weights_sha256": sha256(args.weights),
            "binary_sha256": sha256(args.binary),
            "nnue_output": "residual-material",
        },
        "selection_rule": {
            "both_arms_require_exact_deterministic_calibration": True,
            "fixed_nodes_eligible_if": (
                "its old-depth3 regret class matches depth7 on all three problem parents "
                "and free bestmove agrees on at least 4/6 parents"
            ),
            "if_fixed_nodes_eligible": "choose lower median per-parent calibration elapsed time; ties choose depth7",
            "otherwise": "choose depth7",
        },
        "pass_rule": {
            "selected_contract_labels_major_peer_regret_ge_300cp": 0,
            "selected_contract_labels_major_depth7_regret_ge_300cp": 0,
            "old_depth3_major_depth7_regret_must_strictly_decrease": True,
            "development_match_authorized": False,
            "q20_authorized": False,
        },
        "inputs": {
            "q21n_decision": bind(args.q21n_decision),
            "q21n_summary": bind(args.q21n_summary),
            "shallow": bind(args.shallow),
            "corpus": bind(args.corpus),
            "depth7_measurements": bind(args.depth7_measurements),
            "binary": bind(args.binary),
            "weights": bind(args.weights),
        },
        "tools": {
            "preparer": bind(Path(__file__).resolve()),
            "runner": bind(args.runner),
            "finalizer": bind(args.finalizer),
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--q21n-decision", type=Path, required=True)
    parser.add_argument("--q21n-summary", type=Path, required=True)
    parser.add_argument("--shallow", type=Path, required=True)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--depth7-measurements", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--weights", type=Path, required=True)
    parser.add_argument("--runner", type=Path, required=True)
    parser.add_argument("--finalizer", type=Path, required=True)
    parser.add_argument("--control-seed", type=int, default=2121)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        document = prepare(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"status": document["status"], "parents": document["selection"]["parents"], "arms": document["arms"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
