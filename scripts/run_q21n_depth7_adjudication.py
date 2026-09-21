#!/usr/bin/env python3
"""Run Q21n's preregistered free/fixed-root depth-7 adjudication."""

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


def run(args: argparse.Namespace) -> dict[str, Any]:
    prereg = json.loads(args.preregistration.read_text(encoding="utf-8"))
    corpus = json.loads(args.corpus.read_text(encoding="utf-8"))
    require(
        prereg.get("schema") == "sekirei.q21n-depth7-adjudication-preregistration.v1"
        and prereg.get("status") == "frozen_before_depth7_labels",
        "unexpected depth-7 preregistration",
    )
    require(prereg["inputs"]["corpus"]["sha256"] == sha256(args.corpus), "corpus SHA mismatch")
    require(prereg["contract"]["binary_sha256"] == sha256(args.binary), "binary SHA mismatch")
    require(prereg["contract"]["weights_sha256"] == sha256(args.weights), "weights SHA mismatch")
    positions = {row["id"]: row for row in corpus.get("positions", [])}
    os.environ["RAYON_NUM_THREADS"] = "1"
    rows = []
    for selection in prereg["selected"]:
        position = positions[selection["position_id"]]
        common = {
            "binary": args.binary,
            "sfen": position["initial_sfen"],
            "nodes": 1,
            "timeout": prereg["contract"]["timeout_seconds_per_search"],
            "weights": args.weights,
            "max_depth": prereg["contract"]["depth"],
            "history_moves_usi": position["history_before_usi"],
            "expected_sfen": position["sfen"],
            "nnue_output": prereg["contract"]["nnue_output"],
        }
        free = run_position(**common)
        fixed = {
            move: run_position(**common, root_move=move)
            for move in selection["depth3_top_moves"]
        }
        rows.append({"selection": selection, "free": free, "fixed_depth3_top": fixed})
    return {
        "schema": "sekirei.q21n-depth7-adjudication-measurements.v1",
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
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        document = run(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {len(document['rows'])} adjudicated parents")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
