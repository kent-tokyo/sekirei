#!/usr/bin/env python3
"""Generate resumable, complete depth-3 roots for frozen Q28 train parents."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
from typing import Any

from prepare_q28_training import RUNTIME_SCHEMA, bind, read, require, sha256
from run_core_floodgate_diagnostic import run_position


def atomic_write(path: Path, document: dict[str, Any]) -> None:
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    os.replace(temporary, path)


def exact_root(result: dict[str, Any]) -> bool:
    candidates = result.get("root_candidates")
    return (
        result.get("completion") == "search_completed"
        and result.get("completed_iteration_valid") == "true"
        and result.get("completed_bound") == "exact"
        and result.get("pv_legal") is True
        and result.get("history_matches_expected") == "true"
        and isinstance(candidates, list)
        and result.get("root_legal_move_count") == len(candidates)
        and len(candidates) >= 2
    )


HOLDOUT_SCHEMA = "sekirei.q28-holdout-preregistration.v1"
Q29_RUNTIME_SCHEMA = "sekirei.q29-runtime-preregistration.v1"
Q29_HOLDOUT_SCHEMA = "sekirei.q29-holdout-preregistration.v1"
Q30_RUNTIME_SCHEMA = "sekirei.q30-runtime-preregistration.v1"
Q30_HOLDOUT_SCHEMA = "sekirei.q30-holdout-preregistration.v1"


def phase_contract(prereg: dict[str, Any]) -> None:
    schema = prereg.get("schema")
    status = prereg.get("status")
    if schema == RUNTIME_SCHEMA and status == "frozen_before_shallow_labels":
        return
    if schema == HOLDOUT_SCHEMA and status == "frozen_before_holdout_labels":
        return
    if schema == Q29_HOLDOUT_SCHEMA and status == "frozen_before_holdout_labels":
        return
    if schema == Q29_RUNTIME_SCHEMA and status == "frozen_before_shallow_labels":
        return
    if schema == Q30_RUNTIME_SCHEMA and status == "frozen_before_shallow_labels":
        return
    if schema == Q30_HOLDOUT_SCHEMA and status == "frozen_before_holdout_labels":
        return
    raise ValueError("unexpected Q28 shallow-label preregistration")


def run(args: argparse.Namespace) -> dict[str, Any]:
    prereg = read(args.runtime_preregistration)
    phase_contract(prereg)
    for name, path in (("corpus", args.corpus), ("engine", args.engine), ("weights", args.weights)):
        require(prereg["inputs"][name]["sha256"] == sha256(path), f"{name} SHA mismatch")
    require(prereg["tools"]["shallow_runner"]["sha256"] == sha256(Path(__file__).resolve()), "runner SHA mismatch")
    corpus = read(args.corpus)
    positions = corpus.get("positions", [])
    require(len(positions) == prereg["parents"], "Q28 parent count mismatch")
    output = {
        "schema": "sekirei.root-rank-teacher-corpus.v1",
        "diagnostic_only": True,
        "strength_claim": "not_permitted",
        "contract": {
            "depth": prereg["shallow_contract"]["max_depth"],
            "threads": 1,
            "spec_top_n": 0,
            "root_candidate_mode": "complete_legal_set",
            "root_candidate_limit": prereg["shallow_contract"]["root_candidates"],
            "complete_legal_root_set": True,
            "normal_score_abs_max_cp": prereg["candidate_contract"]["normal_score_abs_max_cp"],
        },
        "teacher": {
            "binary": str(args.engine),
            "binary_sha256": sha256(args.engine),
            "weights": str(args.weights),
            "weights_sha256": sha256(args.weights),
            "nnue_output": prereg["shallow_contract"]["nnue_output"],
        },
        "source_corpus": bind(args.corpus),
        "runtime_preregistration": bind(args.runtime_preregistration),
        "rows": [],
    }
    if args.output.exists():
        prior = read(args.output)
        require(prior.get("source_corpus") == output["source_corpus"], "existing shallow output has another corpus")
        require(prior.get("runtime_preregistration") == output["runtime_preregistration"], "existing shallow output has another runtime")
        output = prior
    completed = {row["id"] for row in output["rows"]}
    os.environ["RAYON_NUM_THREADS"] = "1"
    for position in positions:
        if position["id"] in completed:
            continue
        result = run_position(
            args.engine,
            position["initial_sfen"],
            1,
            prereg["shallow_contract"]["timeout_seconds_per_search"],
            args.weights,
            max_depth=prereg["shallow_contract"]["max_depth"],
            root_candidates=prereg["shallow_contract"]["root_candidates"],
            history_moves_usi=position["history_before_usi"],
            expected_sfen=position["sfen"],
            nnue_output=prereg["shallow_contract"]["nnue_output"],
        )
        require(exact_root(result), f"{position['id']}: incomplete depth-3 root")
        output["rows"].append({
            "id": position["id"], "category": position["category"],
            "initial_sfen": position["initial_sfen"], "history_before_usi": position["history_before_usi"],
            "sfen": position["sfen"], "source": position.get("source"),
            "teacher_root": result, "candidate_prefix_complete": True, "complete_legal_root_set": True,
        })
        atomic_write(args.output, output)
        print(f"Q28 shallow labels: {len(output['rows'])}/{len(positions)} parents", flush=True)
    return output


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runtime-preregistration", type=Path, required=True)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--weights", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        output = run(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    print(f"wrote {args.output}: {len(output['rows'])} parents")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
