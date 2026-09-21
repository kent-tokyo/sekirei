#!/usr/bin/env python3
"""Generate resumable Q21p depth-7 free and fixed-root ranking labels."""

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


def top_moves(
    row: dict[str, Any],
    limit: int,
    score_limit: int,
    *,
    ordinary_only: bool,
) -> list[str]:
    candidates = row.get("teacher_root", {}).get("root_candidates", [])
    eligible = [
        (candidate["move"], candidate["score_cp"])
        for candidate in candidates
        if isinstance(candidate.get("move"), str)
        and isinstance(candidate.get("score_cp"), int)
        and (not ordinary_only or abs(candidate["score_cp"]) <= score_limit)
    ]
    kind = "ordinary shallow" if ordinary_only else "completed shallow"
    require(len(eligible) >= 2, f"{row.get('id')}: fewer than two {kind} candidates")
    return [move for move, _ in sorted(eligible, key=lambda item: (-item[1], item[0]))[:limit]]


def atomic_write(path: Path, document: dict[str, Any]) -> None:
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(
        json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    os.replace(temporary, path)


def search(args: argparse.Namespace, position: dict[str, Any], root_move: str | None) -> dict[str, Any]:
    return run_position(
        args.engine,
        position["initial_sfen"],
        1,
        args.timeout,
        args.weights,
        root_move=root_move,
        max_depth=7,
        history_moves_usi=position["history_before_usi"],
        expected_sfen=position["sfen"],
        nnue_output="residual-material",
    )


def run(args: argparse.Namespace) -> dict[str, Any]:
    prereg = json.loads(args.preregistration.read_text(encoding="utf-8"))
    corpus = json.loads(args.corpus.read_text(encoding="utf-8"))
    shallow = json.loads(args.shallow_teacher.read_text(encoding="utf-8"))
    expected_status = {
        "sekirei.q21p-depth7-ranking-pilot-preregistration.v2": "frozen_before_depth7_labels",
        "sekirei.q21p-depth7-ranking-pilot-preregistration.v3": (
            "frozen_before_depth7_validation_labels_and_candidate_training"
        ),
        "sekirei.q21p-depth7-ranking-pilot-preregistration.v4": (
            "frozen_before_depth7_validation_labels_and_candidate_training"
        ),
        "sekirei.q21t-top-choice-pilot-preregistration.v1": (
            "frozen_before_depth7_labels_and_candidate_training"
        ),
        "sekirei.q21u-listwise-validation-preregistration.v1": (
            "frozen_before_depth7_labels_and_validation_screen"
        ),
    }.get(prereg.get("schema"))
    require(expected_status is not None and prereg.get("status") == expected_status, "unexpected Q21p preregistration")
    for name, path in (
        ("corpus", args.corpus),
        ("shallow_teacher", args.shallow_teacher),
        ("engine", args.engine),
        ("weights", args.weights),
    ):
        require(prereg["inputs"][name]["sha256"] == sha256(path), f"{name} SHA mismatch")
    contract = prereg["candidate_contract"]
    require(args.timeout == prereg["teacher_contract"]["timeout_seconds_per_search"], "timeout differs from preregistration")
    positions = {row["id"]: row for row in corpus["positions"]}
    shallow_rows = {row["id"]: row for row in shallow["rows"]}
    require(set(positions) == set(shallow_rows) and len(positions) == prereg["parents"], "parent set mismatch")
    output = {
        "schema": "sekirei.q21p-depth7-label-measurements.v1",
        "diagnostic_only": True,
        "strength_claim": False,
        "preregistration": {"path": str(args.preregistration), "sha256": sha256(args.preregistration)},
        "rows": [],
    }
    if args.output.is_file():
        prior = json.loads(args.output.read_text(encoding="utf-8"))
        require(
            prior.get("schema") == output["schema"]
            and prior.get("preregistration", {}).get("sha256") == output["preregistration"]["sha256"],
            "existing output belongs to another Q21p run",
        )
        output = prior
    preregistered_runner = prereg["tools"].get("runner") or prereg["tools"].get("label_runner")
    require(isinstance(preregistered_runner, dict), "preregistration lacks a label runner binding")
    output["tool_provenance"] = {
        "preregistered_sha256": preregistered_runner["sha256"],
        "executed_sha256": sha256(Path(__file__).resolve()),
        "contract_preserving_fix_after_partial_measurement": (
            preregistered_runner["sha256"] != sha256(Path(__file__).resolve())
        ),
        "fix_scope": (
            "v3 uses every completed depth-3 top move, including mate-like scores, as the frozen "
            "candidate source; ordinary-cp filtering remains at depth-7 pair construction"
        ),
    }
    completed = {row["id"] for row in output["rows"]}
    os.environ["RAYON_NUM_THREADS"] = "1"
    for identifier, position in positions.items():
        if identifier in completed:
            continue
        shallow_moves = top_moves(
            shallow_rows[identifier],
            contract["top_k_after_depth7_reranking"],
            contract["normal_score_abs_max_cp"],
            ordinary_only=prereg["schema"].endswith(".v2"),
        )
        free = [
            search(args, position, None)
            for _ in range(contract["free_repeats"])
        ]
        free_moves = {
            result.get("bestmove") for result in free if isinstance(result.get("bestmove"), str)
        }
        candidate_moves = sorted(set(shallow_moves) | free_moves)
        require(
            2 <= len(candidate_moves) <= contract["maximum_moves_per_parent"],
            f"{identifier}: candidate union size {len(candidate_moves)} violates contract",
        )
        fixed = [
            {move: search(args, position, move) for move in candidate_moves}
            for _ in range(contract["fixed_root_repeats"])
        ]
        output["rows"].append({
            "id": identifier,
            "category": position["category"],
            "initial_sfen": position["initial_sfen"],
            "history_before_usi": position["history_before_usi"],
            "sfen": position["sfen"],
            "source": position.get("source"),
            "shallow_top8_moves": shallow_moves,
            "candidate_moves": candidate_moves,
            "free": free,
            "fixed": fixed,
        })
        atomic_write(args.output, output)
        print(f"q21p labels: {len(output['rows'])}/{len(positions)} parents", flush=True)
    return output


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preregistration", type=Path, required=True)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--shallow-teacher", type=Path, required=True)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--weights", type=Path, required=True)
    parser.add_argument("--timeout", type=float, default=600)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        document = run(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    print(f"wrote {args.output}: {len(document['rows'])} parents")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
