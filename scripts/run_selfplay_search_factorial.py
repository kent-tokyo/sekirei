#!/usr/bin/env python3
"""Run the bounded C5c3 TT/NMP factorial follow-up on one healthy source."""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("fixed", ROOT / "run_fixed_selfplay_diagnostic.py")
assert SPEC and SPEC.loader
FIXED = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(FIXED)


CONFIGS = {
    "cold_nmp_on": {"warmup_nodes": None, "disable_nmp": False},
    "warm_nmp_on": {"warmup_nodes": 20_000, "disable_nmp": False},
    "cold_nmp_off": {"warmup_nodes": None, "disable_nmp": True},
}


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--classification", type=Path, required=True)
    parser.add_argument("--positions", type=Path, required=True)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--weights", type=Path, required=True)
    parser.add_argument("--nodes", type=int, default=100_000)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    classification = json.loads(args.classification.read_text(encoding="utf-8"))
    if classification.get("summary", {}).get("c5c3_trigger") is not True:
        parser.error("C5c3 is conditional: classifier did not reproduce the registered trigger")
    source = json.loads(args.positions.read_text(encoding="utf-8"))
    if source.get("schema") != "sekirei.selfplay-diagnostic-positions.v1" or source.get("status") != "development_only":
        parser.error("positions must be the healthy-run development-only diagnostic source")
    positions = source.get("positions", [])
    if len(positions) != 32 or any(row.get("already_in_teacher_cache") is not False for row in positions):
        parser.error("C5c3 requires exactly 32 non-overlapping positions")
    results = []
    for index, row in enumerate(positions, 1):
        evaluators = {}
        for evaluator, weights in (("material", None), ("nnue_gate0_init_fix_absolute", args.weights)):
            evaluators[evaluator] = {
                name: FIXED.run_one(args.engine, row, nodes=args.nodes, weights=weights, root_move=None, **config)
                for name, config in CONFIGS.items()
            }
        results.append({"id": f"c5c3-{index:02d}", "source": {key: row[key] for key in ("source_id", "game_number", "ply", "opening", "pre_move_sfen", "categories", "observed_score_cp")}, "results": evaluators})
    document = {
        "schema": "sekirei.selfplay-search-factorial.v1", "diagnostic_only": True, "strength_claim": False,
        "contract": {"nodes": args.nodes, "threads": 1, "spec_top_n": 0, "root_mode": "free_only", "configs": CONFIGS, "one_factor_rule": "warm TT changes only warmup; NMP-off changes only null-move pruning"},
        "inputs": {"classification": str(args.classification), "classification_sha256": sha256(args.classification), "positions": str(args.positions), "positions_sha256": sha256(args.positions), "engine": str(args.engine), "engine_sha256": sha256(args.engine), "weights": str(args.weights), "weights_sha256": sha256(args.weights)},
        "results": results,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"positions": len(results), **document["contract"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
