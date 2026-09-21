#!/usr/bin/env python3
"""Run the preregistered Q21s content-versus-cost audit on Q21r games."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
from pathlib import Path

import run_q21j_failure_audit as base


CORPUS_SCHEMA = "sekirei.q21s-failure-audit-corpus.v1"
PREREG_SCHEMA = "sekirei.q21s-failure-audit-preregistration.v1"
MEASUREMENTS_SCHEMA = "sekirei.q21s-failure-audit-measurements.v1"


def run_search(
    binary: Path,
    candidate: Path,
    teacher: Path,
    position: dict,
    cell: str,
    amount: int,
    arm: str,
    max_depth: int,
) -> dict:
    command = [str(binary), "--profile-cost", "--sfen", position["initial_sfen"]]
    command.extend(("--moves", " ".join(position["history_before_usi"])))
    command.extend(("--expected-sfen", position["sfen"]))
    if cell.startswith("fixed_nodes"):
        command.extend(("--nodes", str(amount), "--max-depth", str(max_depth)))
        # The diagnostic intentionally rejects --disable-root-mate-safety with
        # a forced root move.  Free search needs it to prevent the root safety
        # pass from overwhelming the nominal node budget; forced search has a
        # single root and remains bounded without it.
        if cell == "fixed_nodes_free":
            command.append("--disable-root-mate-safety")
        timeout = 60.0
    else:
        command.extend(("--time-ms", str(amount)))
        timeout = amount / 1000 + 30.0
    if cell == "fixed_nodes_actual":
        command.extend(("--root-move", position["actual_move_usi"]))
    weights, scale = base.arm_options(arm, candidate, teacher)
    if weights is not None:
        command.extend(
            (
                "--weights",
                str(weights),
                "--nnue-output",
                "residual-material",
                "--nnue-residual-scale-permille",
                str(scale),
            )
        )
    env = dict(os.environ)
    env["RAYON_NUM_THREADS"] = "1"
    completed = subprocess.run(
        command, text=True, capture_output=True, check=False, timeout=timeout, env=env
    )
    if completed.returncode:
        raise RuntimeError(
            f"diagnostic failed ({completed.returncode}): {completed.stderr[-2000:]}"
        )
    return base.parse_profile(completed.stdout)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--teacher", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--nodes", type=int, default=100_000)
    parser.add_argument("--time-ms", type=int, default=1_000)
    parser.add_argument("--max-depth", type=int, default=5)
    args = parser.parse_args()
    if args.nodes <= 0 or args.time_ms <= 0 or args.max_depth <= 0:
        parser.error("nodes, time-ms, and max-depth must be positive")
    for path in (args.corpus, args.binary, args.candidate, args.teacher):
        if not path.is_file():
            parser.error(f"missing input: {path}")
    corpus = json.loads(args.corpus.read_text(encoding="utf-8"))
    positions = corpus.get("positions", [])
    if corpus.get("schema") != CORPUS_SCHEMA or len(positions) != 32:
        parser.error("corpus must contain the frozen 32-game Q21s selection")
    by_id = {position["id"]: position for position in positions}
    if len(by_id) != 32:
        parser.error("Q21s position IDs are not unique")

    root = Path(__file__).resolve().parent.parent
    preregistration = {
        "schema": PREREG_SCHEMA,
        "status": "frozen_before_measurement",
        "diagnostic_only": True,
        "strength_claim": False,
        "question": (
            "explain why Q21p's static rank-loss gain did not transfer in Q21r "
            "by separating evaluator content from residual-NNUE compute cost"
        ),
        "contract": {
            "fixed_nodes": args.nodes,
            "fixed_nodes_max_depth": args.max_depth,
            "fixed_nodes_root_mate_safety": {
                "free": False,
                "forced_actual": True,
                "reason": (
                    "the diagnostic forbids disabling root mate safety with --root-move; "
                    "forced search has only one root move"
                ),
            },
            "fixed_nodes_budget_semantics": (
                "node limit is checked between completed iterations; max-depth bounds iteration "
                "overshoot and all arms use the same contract"
            ),
            "fixed_time_ms": args.time_ms,
            "fixed_time_root_mate_safety": True,
            "free_repetitions": 3,
            "forced_actual_repetitions": 1,
            "rayon_threads": 1,
            "arms": {
                "material": "no weights",
                "candidate-cost-only": "Q21p candidate, residual-material scale=0",
                "candidate": "Q21p candidate, residual-material scale=1000",
                "teacher": "fixed T, residual-material scale=1000",
            },
            "cost_identity_requirement": (
                "material and candidate-cost-only fixed-node score/bestmove match >=95%"
            ),
            "cost_pressure_signal": (
                "median fixed-time candidate-cost-only/material node ratio <=0.75"
            ),
            "content_signal": (
                "fixed-node candidate/material bestmove disagreement or >=300cp score difference; "
                "report actual-move regret separately"
            ),
            "attribution_boundary": (
                "classify cost and content signals separately; do not infer a causal percentage "
                "of Q21r losses"
            ),
        },
        "inputs": {
            "corpus": {"path": str(args.corpus), "sha256": base.sha256(args.corpus)},
            "binary": {"path": str(args.binary), "sha256": base.sha256(args.binary)},
            "candidate": {"path": str(args.candidate), "sha256": base.sha256(args.candidate)},
            "teacher": {"path": str(args.teacher), "sha256": base.sha256(args.teacher)},
            "runner_source": {
                "path": str(Path(__file__).resolve()),
                "sha256": base.sha256(Path(__file__).resolve()),
            },
        },
        "position_ids": list(by_id),
        "git": base.git_metadata(root),
    }
    args.output_dir.mkdir(parents=True, exist_ok=True)
    prereg_path = args.output_dir / "preregistration.json"
    encoded = json.dumps(preregistration, indent=2, sort_keys=True) + "\n"
    if prereg_path.exists() and prereg_path.read_text(encoding="utf-8") != encoded:
        parser.error("existing preregistration differs; use a fresh output directory")
    prereg_path.write_text(encoded, encoding="utf-8")

    results_path = args.output_dir / "measurements.jsonl"
    try:
        existing = base.load_existing(results_path)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        parser.error(str(error))
    plan = base.measurement_plan(list(by_id))
    for index, item in enumerate(plan, 1):
        item_key = base.key(item)
        if item_key in existing:
            continue
        amount = args.time_ms if item["cell"] == "fixed_time_free" else args.nodes
        result = run_search(
            args.binary,
            args.candidate,
            args.teacher,
            by_id[item["position_id"]]["position"],
            item["cell"],
            amount,
            item["arm"],
            args.max_depth,
        )
        row = {
            **item,
            "amount": amount,
            "game_num": by_id[item["position_id"]]["game_num"],
            "game_result": by_id[item["position_id"]]["game_result"],
            "actual_move_usi": by_id[item["position_id"]]["position"]["actual_move_usi"],
            "result": result,
        }
        base.append_result(results_path, row)
        existing[item_key] = row
        if index % 32 == 0 or index == len(plan):
            print(f"progress {len(existing)}/{len(plan)}", flush=True)
    manifest = {
        "schema": MEASUREMENTS_SCHEMA,
        "status": "complete",
        "diagnostic_only": True,
        "strength_claim": False,
        "preregistration_sha256": base.sha256(prereg_path),
        "measurements": len(existing),
        "expected_measurements": len(plan),
        "measurements_path": str(results_path),
        "measurements_sha256": base.sha256(results_path),
    }
    (args.output_dir / "manifest.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
