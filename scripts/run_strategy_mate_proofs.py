#!/usr/bin/env python3
"""Apply Sekirei's complete bounded mate analyzer to composed candidates.

The proof concerns the opponent's checking mate after each candidate move.
`no_mate` excludes only the requested ply bound; neither result is a general
best-move or playing-strength label. One fresh process is used per candidate.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import time
from pathlib import Path

from run_tactical_line_diagnostic import atomic_json, load_cases, run_process, sha256


def parse_proof_fields(stdout: str) -> dict[str, str]:
    lines = stdout.splitlines()
    if len(lines) != 1 or not lines[0].startswith("move="):
        raise ValueError("mate-proof binary must emit one move result line")
    fields = dict(field.split("=", 1) for field in lines[0].split("\t"))
    required = {"move", "initial_hash", "child_hash", "max_ply", "node_limit", "outcome",
                "aborted", "abort_reason", "invalid_reason", "mate_ply",
                "solutions", "nodes", "distance_rule"}
    if required - fields.keys():
        raise ValueError(f"mate-proof result missing {sorted(required - fields.keys())}")
    return fields


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cases", type=Path, required=True)
    parser.add_argument("--proof-bin", type=Path, required=True)
    parser.add_argument("--max-ply", type=int, default=3)
    parser.add_argument("--nodes", type=int, default=10_000)
    parser.add_argument("--max-invocations", type=int, required=True)
    parser.add_argument("--max-total-seconds", type=int, default=90)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if not args.proof_bin.is_file():
        parser.error(f"proof binary does not exist: {args.proof_bin}")
    if args.max_ply not in (1, 3, 5, 7) or not 1 <= args.nodes <= 100_000:
        parser.error("--max-ply must be 1, 3, 5 or 7; --nodes must be 1..=100000")
    if not 1 <= args.max_invocations <= 24 or not 1 <= args.max_total_seconds <= 90:
        parser.error("--max-invocations must be 1..=24 and --max-total-seconds 1..=90")
    cases = load_cases(args.cases)
    tasks = []
    for case in cases["cases"]:
        for stage in case["stages"]:
            if stage.get("moves") or stage.get("expected_sfen", case["sfen"]) != case["sfen"]:
                parser.error("this proof runner accepts only direct, unreplayed case positions")
            tasks.extend((case["id"], stage["id"], case["sfen"], move)
                         for move in stage.get("candidates", []))
    if not tasks:
        parser.error("no candidate moves")
    if len(tasks) > args.max_invocations:
        parser.error(f"batch requires {len(tasks)} invocations")

    deadline = time.monotonic() + args.max_total_seconds
    rows = []
    status = "complete"
    failure = None
    for case_id, stage_id, sfen, move in tasks:
        try:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("mate-proof batch deadline exhausted")
            command = [str(args.proof_bin), "--sfen", sfen, "--move", move,
                       "--max-ply", str(args.max_ply), "--nodes", str(args.nodes)]
            fields = parse_proof_fields(run_process(command, timeout=remaining).stdout)
            if (fields["move"] != move or fields["max_ply"] != str(args.max_ply)
                    or fields["node_limit"] != str(args.nodes)):
                raise ValueError("mate-proof binary returned a mismatched input")
            if fields["outcome"] not in ("mate", "no_mate", "unknown"):
                raise ValueError("mate-proof binary returned an invalid outcome")
            rows.append({
                "case_id": case_id,
                "stage_id": stage_id,
                "candidate": move,
                "initial_hash": fields["initial_hash"],
                "child_hash": fields["child_hash"],
                "outcome": fields["outcome"],
                "aborted": fields["aborted"] == "true",
                "abort_reason": fields["abort_reason"],
                "invalid_reason": fields["invalid_reason"],
                "mate_ply": int(fields["mate_ply"]),
                "solutions": [item for item in fields["solutions"].split(",") if item],
                "nodes": int(fields["nodes"]),
                "distance_rule": fields["distance_rule"],
            })
        except (subprocess.TimeoutExpired, TimeoutError, RuntimeError, ValueError) as error:
            status = "interrupted"
            failure = {"case_id": case_id, "stage_id": stage_id, "candidate": move,
                       "reason": type(error).__name__, "detail": str(error)[:500]}
            break
    report = {
        "schema": "sekirei.strategy-bounded-mate-proofs.v1",
        "diagnostic_only": True,
        "strength_claim": False,
        "scope": "opponent_checking_mate_after_candidate_within_max_ply",
        "status": status,
        "completed_invocations": len(rows),
        "planned_invocations": len(tasks),
        "failure": failure,
        "inputs": {
            "cases": str(args.cases), "cases_sha256": sha256(args.cases),
            "proof_bin": str(args.proof_bin), "proof_bin_sha256": sha256(args.proof_bin),
            "max_ply": args.max_ply, "nodes_per_candidate": args.nodes,
            "max_invocations": args.max_invocations,
            "max_total_seconds": args.max_total_seconds,
        },
        "rows": rows,
    }
    atomic_json(args.output, report)
    print(json.dumps({"rows": len(rows), "status": status,
                      "output": str(args.output)}, sort_keys=True))
    return 0 if status == "complete" else 1


if __name__ == "__main__":
    raise SystemExit(main())
