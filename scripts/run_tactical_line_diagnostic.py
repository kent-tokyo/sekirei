#!/usr/bin/env python3
"""Measure first moves, replies, and responses as separate fixed-node facts.

The case file may attach human or published reference moves, but they remain
metadata: this tool never treats them as a correctness oracle.  Every normal
or fixed-root search starts a fresh diagnostic process, so TT, history tables,
and worker-local state cannot leak between alternatives.  The requested node
budget applies to each invocation (and therefore to each fixed candidate),
not to the case as a whole.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import tempfile
import time
from pathlib import Path
from typing import Any

from diagnostic_contract import score_kind
from run_fixed_selfplay_diagnostic import parse_fields


SCHEMA = "sekirei.tactical-line-cases.v1"
REPORT_SCHEMA = "sekirei.tactical-line-diagnostic.v1"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_cases(path: Path) -> dict[str, Any]:
    document = json.loads(path.read_text(encoding="utf-8"))
    if document.get("schema") != SCHEMA:
        raise ValueError(f"case schema must be {SCHEMA}")
    cases = document.get("cases")
    if not isinstance(cases, list) or not cases:
        raise ValueError("cases must be a non-empty list")
    case_ids: set[str] = set()
    for case in cases:
        case_id = case.get("id")
        sfen = case.get("sfen")
        stages = case.get("stages")
        if not isinstance(case_id, str) or not case_id or case_id in case_ids:
            raise ValueError("case ids must be unique non-empty strings")
        case_ids.add(case_id)
        if not isinstance(sfen, str) or len(sfen.split()) != 4:
            raise ValueError(f"{case_id}: sfen must have four fields")
        if not isinstance(stages, list) or not stages:
            raise ValueError(f"{case_id}: stages must be a non-empty list")
        stage_ids: set[str] = set()
        for stage in stages:
            stage_id = stage.get("id")
            if not isinstance(stage_id, str) or not stage_id or stage_id in stage_ids:
                raise ValueError(f"{case_id}: stage ids must be unique non-empty strings")
            stage_ids.add(stage_id)
            for key in ("moves", "candidates", "reference_moves"):
                values = stage.get(key, [])
                if not isinstance(values, list) or not all(
                    isinstance(value, str) and value for value in values
                ):
                    raise ValueError(f"{case_id}/{stage_id}: {key} must be a string list")
            candidates = stage.get("candidates", [])
            if len(candidates) != len(set(candidates)):
                raise ValueError(f"{case_id}/{stage_id}: candidates must be unique")
            if not set(stage.get("reference_moves", [])).issubset(candidates):
                raise ValueError(
                    f"{case_id}/{stage_id}: reference moves must also be explicit candidates"
                )
            expected = stage.get("expected_sfen")
            if expected is not None and (
                not isinstance(expected, str) or len(expected.split()) != 4
            ):
                raise ValueError(f"{case_id}/{stage_id}: expected_sfen must have four fields")
    return document


def run_process(command: list[str], timeout: int = 180,
                env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(
        command, text=True, capture_output=True, check=False, timeout=timeout, env=env
    )
    if completed.returncode:
        detail = completed.stderr.strip() or completed.stdout.strip()
        raise RuntimeError(f"command failed ({completed.returncode}): {detail[-1200:]}")
    return completed


def remaining_timeout(deadline: float | None) -> float:
    if deadline is None:
        return 180
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise TimeoutError("diagnostic batch deadline exhausted")
    return min(180, remaining)


def materialize_position(
    helper: Path, initial_sfen: str, moves: list[str], expected_sfen: str | None,
    *, deadline: float | None = None,
) -> dict[str, str]:
    command = [str(helper), "--sfen", initial_sfen, "--moves", " ".join(moves)]
    completed = run_process(command, timeout=remaining_timeout(deadline))
    fields = dict(field.split("=", 1) for field in completed.stdout.strip().split("\t"))
    if set(fields) != {"sfen", "hash"}:
        raise ValueError("history helper returned an unexpected contract")
    if expected_sfen is not None and fields["sfen"] != expected_sfen:
        raise ValueError(
            f"replayed stage differs from expected SFEN: {fields['sfen']} != {expected_sfen}"
        )
    return fields


def side_to_move(initial_sfen: str, moves: list[str]) -> str:
    initial = initial_sfen.split()[1]
    black = (initial == "b") == (len(moves) % 2 == 0)
    return "black" if black else "white"


def encode_result(fields: dict[str, str], requested_root: str | None) -> dict[str, Any]:
    bestmove = fields["bestmove"]
    if requested_root is not None and bestmove != requested_root:
        raise ValueError(
            f"fixed-root search returned {bestmove!r}, expected {requested_root!r}"
        )
    score = int(fields["score_cp"])
    pv = [move for move in fields["pv_usi"].split(",") if move]
    pv_root = pv[0] if pv else "none"
    matches_pv_root = int(fields["depth"]) > 0 and bestmove == pv_root
    if fields.get("pv_root", pv_root) != pv_root:
        raise ValueError("search diagnostic returned inconsistent PV root")
    completed_bestmove = fields.get("completed_bestmove")
    if completed_bestmove is not None and completed_bestmove != pv_root:
        raise ValueError("search diagnostic returned inconsistent completed bestmove")
    if fields.get("bestmove_matches_pv_root", str(matches_pv_root).lower()) != str(matches_pv_root).lower():
        raise ValueError("search diagnostic returned inconsistent bestmove/PV flag")
    return {
        "requested_root": requested_root,
        "bestmove": bestmove,
        "score_cp": score,
        "score_kind": score_kind(score),
        "depth": int(fields["depth"]),
        "nodes": int(fields["nodes"]),
        "elapsed_ms": int(fields["elapsed_ms"]),
        "bound": fields["bound"],
        "completed_bound": fields["completed_bound"],
        "completed_iteration_valid": fields["completed_iteration_valid"] == "true",
        "aborted": fields["aborted"] == "true",
        "abort_reason": fields["abort_reason"],
        "pv_usi": pv,
        "pv_root": pv_root,
        "completed_bestmove": completed_bestmove,
        "bestmove_matches_pv_root": matches_pv_root,
        "pv_legal": fields["pv_legal"] == "true",
        "history_matches_expected": fields["history_matches_expected"] == "true",
    }


def search(
    engine: Path,
    initial_sfen: str,
    moves: list[str],
    expected_sfen: str,
    nodes: int,
    weights: Path | None,
    nnue_output: str,
    root_move: str | None,
    *,
    disable_lmr: bool = False,
    pruning_counts: bool = False,
    deadline: float | None = None,
) -> dict[str, Any]:
    command = [
        str(engine),
        "--nodes",
        str(nodes),
        "--sfen",
        initial_sfen,
        "--moves",
        " ".join(moves),
        "--expected-sfen",
        expected_sfen,
    ]
    if weights is not None:
        command.extend(["--weights", str(weights), "--nnue-output", nnue_output])
    if root_move is not None:
        command.extend(["--root-move", root_move])
    if disable_lmr:
        command.append("--disable-lmr")
    if pruning_counts:
        command.append("--pruning-counts")
    fields = parse_fields(run_process(command, timeout=remaining_timeout(deadline)).stdout)
    result = encode_result(fields, root_move)
    if pruning_counts:
        keys = (
            "v2_lmp_skips", "v2_quiet_history_skips", "v2_quiet_futility_skips",
            "v2_quiet_see_skips", "v2_tactical_futility_skips",
            "v2_tactical_see_skips", "v2_reduced_children",
        )
        result["v2_move_decisions"] = {key: int(fields[key]) for key in keys}
    return result


def rank_candidates(results: list[dict[str, Any]]) -> None:
    complete = all(
        result["completed_iteration_valid"]
        and result["completed_bound"] == "exact"
        and result["pv_legal"]
        and result["bestmove_matches_pv_root"]
        for result in results
    )
    order = sorted(range(len(results)), key=lambda index: results[index]["score_cp"], reverse=True)
    for rank, index in enumerate(order, 1):
        results[index]["diagnostic_rank"] = rank if complete else None


def atomic_json(path: Path, document: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(
        mode="w", encoding="utf-8", dir=path.parent, delete=False
    ) as temporary:
        json.dump(document, temporary, ensure_ascii=False, indent=2, sort_keys=True)
        temporary.write("\n")
        temporary_path = Path(temporary.name)
    os.replace(temporary_path, path)


def collect_rows(
    cases: dict[str, Any], args: argparse.Namespace, budgets: list[int],
    deadline: float | None, rows: list[dict[str, Any]], active: dict[str, Any],
) -> None:
    for case in cases["cases"]:
        for stage in case["stages"]:
            moves = stage.get("moves", [])
            active.clear()
            active.update({"case_id": case["id"], "stage_id": stage["id"], "moves": moves})
            position = materialize_position(
                args.history_sfen, case["sfen"], moves, stage.get("expected_sfen"),
                deadline=deadline,
            )
            active.update({"position_sfen": position["sfen"], "position_hash": position["hash"]})
            for nodes in budgets:
                active["nodes_per_invocation"] = nodes
                active.pop("unrestricted", None)
                active["fixed_root_candidates"] = []
                unrestricted = search(
                    args.engine, case["sfen"], moves, position["sfen"], nodes,
                    args.weights, args.nnue_output, None,
                    disable_lmr=args.disable_lmr, pruning_counts=args.pruning_counts,
                    deadline=deadline,
                )
                active["unrestricted"] = unrestricted
                candidates = []
                for candidate in stage.get("candidates", []):
                    result = search(
                        args.engine, case["sfen"], moves, position["sfen"], nodes,
                        args.weights, args.nnue_output, candidate,
                        disable_lmr=args.disable_lmr, pruning_counts=args.pruning_counts,
                        deadline=deadline,
                    )
                    candidates.append(result)
                    active["fixed_root_candidates"].append(result)
                rank_candidates(candidates)
                rows.append({
                    "case_id": case["id"],
                    "stage_id": stage["id"],
                    "stage_kind": stage.get("kind", "observation"),
                    "moves": moves,
                    "position_sfen": position["sfen"],
                    "position_hash": position["hash"],
                    "score_perspective": f"{side_to_move(case['sfen'], moves)}_to_move",
                    "nodes_per_invocation": nodes,
                    "reference_moves": stage.get("reference_moves", []),
                    "reference_policy": "metadata_only_not_a_correctness_oracle",
                    "unrestricted": unrestricted,
                    "fixed_root_candidates": candidates,
                })
                active.clear()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cases", type=Path, required=True)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--history-sfen", type=Path, required=True)
    parser.add_argument("--nodes", type=int, action="append", dest="budgets")
    parser.add_argument("--max-invocations", type=int, help="reject a batch before starting if its history and search subprocess count exceeds this limit")
    parser.add_argument("--max-total-seconds", type=int, help="hard deadline for the whole batch (1..=90); save completed and partial work on timeout")
    parser.add_argument("--disable-lmr", action="store_true", help="diagnostic ablation: disable root and interior late-move reductions")
    parser.add_argument("--pruning-counts", action="store_true", help="record aggregate SEARCH_V2 move rejection counts without per-node timers")
    parser.add_argument("--weights", type=Path)
    parser.add_argument(
        "--nnue-output", choices=("absolute", "residual-material"), default="absolute"
    )
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    budgets = args.budgets or [100_000]
    if any(nodes <= 0 for nodes in budgets) or len(budgets) != len(set(budgets)):
        parser.error("--nodes values must be positive and unique")
    for executable in (args.engine, args.history_sfen):
        if not executable.is_file():
            parser.error(f"executable does not exist: {executable}")
    if args.weights is not None and not args.weights.is_file():
        parser.error(f"weights do not exist: {args.weights}")
    if args.max_total_seconds is not None and not 1 <= args.max_total_seconds <= 90:
        parser.error("--max-total-seconds must be 1..=90")

    cases = load_cases(args.cases)
    planned_invocations = sum(
        1 + len(budgets) * (1 + len(stage.get("candidates", [])))
        for case in cases["cases"]
        for stage in case["stages"]
    )
    if args.max_invocations is not None:
        if args.max_invocations <= 0:
            parser.error("--max-invocations must be positive")
        if planned_invocations > args.max_invocations:
            parser.error(
                f"batch requires {planned_invocations} subprocesses, "
                f"exceeding --max-invocations={args.max_invocations}"
            )
    rows: list[dict[str, Any]] = []
    active: dict[str, Any] = {}
    status = "complete"
    failure = None
    deadline = time.monotonic() + args.max_total_seconds if args.max_total_seconds else None
    try:
        collect_rows(cases, args, budgets, deadline, rows, active)
    except (subprocess.TimeoutExpired, TimeoutError, RuntimeError, ValueError) as error:
        status = "interrupted"
        failure = {"reason": type(error).__name__, "detail": str(error)[:500]}

    document = {
        "schema": REPORT_SCHEMA,
        "diagnostic_only": True,
        "strength_claim": False,
        "status": status,
        "planned_invocations": planned_invocations,
        "completed_rows": len(rows),
        "failure": failure,
        "partial_row": dict(active) if active else None,
        "contract": {
            "budget_scope": "per_fresh_process_per_candidate",
            "threads": 1,
            "spec_top_n": 0,
            "state_isolation": "fresh_process_per_search",
            "disable_lmr": args.disable_lmr,
            "pruning_counts": args.pruning_counts,
            "candidate_and_reanalysis_are_distinct_measurements": True,
        },
        "inputs": {
            "cases": str(args.cases),
            "cases_sha256": sha256(args.cases),
            "engine": str(args.engine),
            "engine_sha256": sha256(args.engine),
            "history_sfen": str(args.history_sfen),
            "history_sfen_sha256": sha256(args.history_sfen),
            "evaluator": "nnue" if args.weights else "material",
            "weights": str(args.weights) if args.weights else None,
            "weights_sha256": sha256(args.weights) if args.weights else None,
            "nnue_output": args.nnue_output if args.weights else None,
            "budgets": budgets,
            "max_total_seconds": args.max_total_seconds,
        },
        "rows": rows,
    }
    atomic_json(args.output, document)
    print(json.dumps({"rows": len(rows), "status": status,
                      "output": str(args.output)}, sort_keys=True))
    return 0 if status == "complete" else 1


if __name__ == "__main__":
    raise SystemExit(main())
