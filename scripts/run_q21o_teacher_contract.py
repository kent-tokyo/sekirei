#!/usr/bin/env python3
"""Run Q21o's preregistered teacher-contract calibration and rank audit."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
from typing import Any

from run_core_floodgate_diagnostic import run_position


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def search(args: argparse.Namespace, position: dict[str, Any], arm: dict[str, Any], root_move: str | None = None) -> dict[str, Any]:
    return run_position(
        args.binary,
        position["initial_sfen"],
        arm["nodes"] if arm["nodes"] is not None else 1,
        args.timeout,
        args.weights,
        root_move=root_move,
        max_depth=arm["max_depth"],
        history_moves_usi=position["history_before_usi"],
        expected_sfen=position["sfen"],
        nnue_output="residual-material",
    )


def run(args: argparse.Namespace) -> dict[str, Any]:
    prereg = json.loads(args.preregistration.read_text(encoding="utf-8"))
    corpus = json.loads(args.corpus.read_text(encoding="utf-8"))
    require(
        prereg.get("schema") == "sekirei.q21o-teacher-contract-preregistration.v1"
        and prereg.get("status") == "frozen_before_measurement",
        "unexpected Q21o preregistration",
    )
    require(prereg["inputs"]["corpus"]["sha256"] == sha256(args.corpus), "corpus SHA mismatch")
    require(prereg["measurement_contract"]["binary_sha256"] == sha256(args.binary), "binary SHA mismatch")
    require(prereg["measurement_contract"]["weights_sha256"] == sha256(args.weights), "weights SHA mismatch")
    require(args.timeout == prereg["measurement_contract"]["timeout_seconds_per_search"], "timeout differs from preregistration")
    positions = {row["id"]: row for row in corpus.get("positions", [])}
    os.environ["RAYON_NUM_THREADS"] = "1"
    rows = []
    for selection in prereg["selection"]["parents"]:
        position = positions[selection["position_id"]]
        calibration: dict[str, list[dict[str, Any]]] = {}
        for arm_name, arm in prereg["arms"].items():
            repeats = []
            for _ in range(prereg["measurement_contract"]["calibration_repeats"]):
                repeats.append({
                    "free": search(args, position, arm),
                    "fixed_depth3_top": {
                        move: search(args, position, arm, move)
                        for move in selection["depth3_top_moves"]
                    },
                })
            calibration[arm_name] = repeats
        candidate_moves = set(selection["depth3_top8_moves"])
        candidate_moves.update(
            repeat["free"].get("bestmove")
            for repeats in calibration.values()
            for repeat in repeats
            if isinstance(repeat["free"].get("bestmove"), str)
        )
        ranking = {}
        for arm_name, arm in prereg["arms"].items():
            first_fixed = calibration[arm_name][0]["fixed_depth3_top"]
            ranking[arm_name] = {
                move: first_fixed.get(move) or search(args, position, arm, move)
                for move in sorted(candidate_moves)
            }
        rows.append({
            "selection": selection,
            "candidate_moves": sorted(candidate_moves),
            "calibration": calibration,
            "ranking": ranking,
        })
    return {
        "schema": "sekirei.q21o-teacher-contract-measurements.v1",
        "diagnostic_only": True,
        "strength_claim": False,
        "preregistration": {"path": str(args.preregistration), "sha256": sha256(args.preregistration)},
        "rows": rows,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preregistration", type=Path, required=True)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--weights", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=600)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        document = run(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {len(document['rows'])} parents")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
