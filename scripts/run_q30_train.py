#!/usr/bin/env python3
"""Train Q30's frozen base and reduced NNUE arms on identical rank pairs."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
from pathlib import Path
from typing import Any

import run_q21u_train_screen as common


SCHEMA = "sekirei.q30-train-preregistration.v1"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def train(
    *, name: str, trainer: Path, auditor: Path, initial: Path, pairs: Path,
    contract: dict[str, Any], output_dir: Path, timeout: float,
) -> dict[str, Any]:
    output_dir.mkdir(parents=True, exist_ok=True)
    baseline_audit = common.audit(auditor, pairs, initial)
    common.atomic_write(output_dir / "baseline-audit.json", baseline_audit)
    candidate = output_dir / "candidate.bin"
    metadata = candidate.with_suffix(".ranking.json")
    if not candidate.exists():
        command = [
            str(trainer), "--ranking-pairs", str(pairs),
            "--ranking-objective", contract["objective"],
            "--ranking-temperature-cp", str(contract["temperature_cp"]),
            "--ranking-parent-balanced", "--ranking-batch-pairs", str(contract["ranking_batch_pairs"]),
            "--ranking-epochs", str(contract["epochs"]), "--init-weights", str(initial),
            "--nnue-output", contract["nnue_output"], "--lr", str(contract["learning_rate"]),
            "--seed", str(contract["seed"]), "--output", str(candidate),
        ]
        completed = subprocess.run(
            command, text=True, capture_output=True, check=False, timeout=timeout,
            env={**os.environ, "RAYON_NUM_THREADS": "1"},
        )
        if completed.returncode:
            raise RuntimeError(f"Q30 {name} training failed: {completed.stderr[-4000:]}")
    training = common.read(metadata)
    require(
        training.get("ranking_objective") == contract["objective"]
        and training.get("ranking_temperature_cp") == contract["temperature_cp"]
        and training.get("epochs") == contract["epochs"],
        f"Q30 {name} training metadata mismatch",
    )
    candidate_audit = common.audit(auditor, pairs, candidate)
    common.atomic_write(output_dir / "candidate-audit.json", candidate_audit)
    return {
        "initial": common.bind(initial),
        "baseline": common.metrics(baseline_audit),
        "candidate": {"artifact": common.bind(candidate), "metrics": common.metrics(candidate_audit)},
        "training": common.bind(metadata),
        "trainer": common.bind(trainer),
        "ranking_auditor": common.bind(auditor),
    }


def run(args: argparse.Namespace) -> dict[str, Any]:
    prereg = common.read(args.preregistration)
    pairs = common.read(args.train_pairs)
    audit = common.read(args.pair_audit)
    require(
        prereg.get("schema") == SCHEMA and prereg.get("status") == "frozen_before_train_labels",
        "unexpected Q30 train preregistration",
    )
    require(
        pairs.get("schema") == "sekirei.root-rank-pairs.v1" and pairs.get("pair_selection") == "top-vs-rest",
        "Q30 requires direct top-versus-rest pairs",
    )
    parent_ids = {row["parent_id"] for row in pairs.get("pairs", [])}
    require(
        audit.get("parents") == prereg["parents"]
        and audit.get("parents_with_pairs") == len(parent_ids)
        and audit.get("parents_excluded") == prereg["parents"] - len(parent_ids),
        "Q30 pair coverage does not account for every frozen train parent",
    )
    bindings = {
        "base_trainer": args.base_trainer,
        "base_ranking_auditor": args.base_ranking_auditor,
        "base_initial_weights": args.base_initial_weights,
        "reduced_trainer": args.reduced_trainer,
        "reduced_ranking_auditor": args.reduced_ranking_auditor,
        "reduced_initial_weights": args.reduced_initial_weights,
    }
    for name, path in bindings.items():
        require(prereg["inputs"][name]["sha256"] == common.sha256(path), f"Q30 {name} SHA mismatch")
    contract = prereg["training_contract"]
    args.output_dir.mkdir(parents=True, exist_ok=True)
    arms = {
        "base_256_32": train(
            name="base_256_32", trainer=args.base_trainer, auditor=args.base_ranking_auditor,
            initial=args.base_initial_weights, pairs=args.train_pairs, contract=contract,
            output_dir=args.output_dir / "base_256_32", timeout=args.timeout,
        ),
        "reduced_128_16": train(
            name="reduced_128_16", trainer=args.reduced_trainer, auditor=args.reduced_ranking_auditor,
            initial=args.reduced_initial_weights, pairs=args.train_pairs, contract=contract,
            output_dir=args.output_dir / "reduced_128_16", timeout=args.timeout,
        ),
    }
    decision = {
        "schema": "sekirei.q30-train-decision.v1",
        "status": "candidates_frozen",
        "diagnostic_only": True,
        "strength_claim": False,
        "holdout_inspected": False,
        "holdout_labels_generated": False,
        "candidate_adopted": False,
        "q27_authorized": False,
        "preregistration": common.bind(args.preregistration),
        "train_pairs": common.bind(args.train_pairs),
        "pair_audit": common.bind(args.pair_audit),
        "arms": arms,
        "next_action": "freeze both arm SHAs before any Q30 holdout label",
        "runner": common.bind(Path(__file__).resolve()),
    }
    common.atomic_write(args.output_dir / "train-decision.json", decision)
    return decision


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preregistration", type=Path, required=True)
    parser.add_argument("--train-pairs", type=Path, required=True)
    parser.add_argument("--pair-audit", type=Path, required=True)
    parser.add_argument("--base-trainer", type=Path, required=True)
    parser.add_argument("--base-ranking-auditor", type=Path, required=True)
    parser.add_argument("--base-initial-weights", type=Path, required=True)
    parser.add_argument("--reduced-trainer", type=Path, required=True)
    parser.add_argument("--reduced-ranking-auditor", type=Path, required=True)
    parser.add_argument("--reduced-initial-weights", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=3600.0)
    args = parser.parse_args()
    try:
        decision = run(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError, RuntimeError) as error:
        parser.error(str(error))
    print(json.dumps({"status": decision["status"], "arms": list(decision["arms"])}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
