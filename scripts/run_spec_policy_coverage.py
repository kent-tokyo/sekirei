#!/usr/bin/env python3
"""Compare static legal speculative top-N with bound baseline moves.

The input is already a diagnostic baseline-move case file. Inclusion is a
candidate-coverage fact, not evidence that speculation helps useful work.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import time
from pathlib import Path

from run_tactical_line_diagnostic import atomic_json, load_cases, run_process, sha256


FIELDS = {"target", "initial_hash", "legal_count", "top_n", "top_moves",
          "target_in_top_n", "target_rank", "tt_mode", "tt_hint", "search_started", "board_restored"}


def parse_probe(stdout: str, target: str, top_n: int, tt_hint: str | None = None) -> dict:
    lines = stdout.splitlines()
    if len(lines) != 1:
        raise ValueError("policy probe must emit exactly one line")
    fields = dict(item.split("=", 1) for item in lines[0].split("\t"))
    if set(fields) != FIELDS or fields["target"] != target or fields["top_n"] != str(top_n):
        raise ValueError("policy probe returned mismatched fields or target")
    expected_mode = "seeded_root_hint" if tt_hint else "empty"
    if (fields["tt_mode"] != expected_mode or fields["tt_hint"] != (tt_hint or "none")
            or fields["search_started"] != "false"
            or fields["board_restored"] != "true"):
        raise ValueError("policy probe violated static empty-TT contract")
    if len(fields["initial_hash"]) != 16 or any(
        char not in "0123456789abcdef" for char in fields["initial_hash"]
    ):
        raise ValueError("policy probe returned invalid hash")
    legal_count = int(fields["legal_count"])
    moves = [move for move in fields["top_moves"].split(",") if move]
    rank = int(fields["target_rank"])
    hit = fields["target_in_top_n"]
    if (legal_count < 1 or len(moves) != min(top_n, legal_count)
            or len(moves) != len(set(moves)) or hit not in ("true", "false")):
        raise ValueError("policy probe returned invalid candidate set")
    expected_rank = moves.index(target) + 1 if target in moves else 0
    if rank != expected_rank or (hit == "true") != (rank > 0):
        raise ValueError("policy probe returned inconsistent target rank")
    return {"target": target, "initial_hash": fields["initial_hash"],
            "legal_count": legal_count, "top_n": top_n, "top_moves": moves,
            "target_in_top_n": rank > 0, "target_rank": rank,
            "tt_mode": expected_mode, "tt_hint": tt_hint}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cases", required=True, type=Path)
    parser.add_argument("--probe-bin", required=True, type=Path)
    parser.add_argument("--top-n", required=True, type=int)
    parser.add_argument("--max-invocations", required=True, type=int)
    parser.add_argument("--max-total-seconds", default=30, type=int)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if not args.probe_bin.is_file():
        parser.error(f"probe binary does not exist: {args.probe_bin}")
    if not 1 <= args.top_n <= 16 or not 1 <= args.max_invocations <= 24:
        parser.error("--top-n must be 1..=16 and --max-invocations 1..=24")
    if not 1 <= args.max_total_seconds <= 90:
        parser.error("--max-total-seconds must be 1..=90")
    cases = load_cases(args.cases)
    if (cases.get("diagnostic_only") is not True or cases.get("strength_claim") is not False
            or not isinstance(cases.get("nodes_per_baseline_search"), int)
            or cases["nodes_per_baseline_search"] <= 0
            or any(not isinstance(cases.get(key), str) or len(cases[key]) != 64
                   or any(char not in "0123456789abcdef" for char in cases[key])
                   for key in ("source_cases_sha256", "source_report_sha256"))):
        parser.error("input must be a bound diagnostic baseline-move case file")
    tasks = []
    for case in cases["cases"]:
        for stage in case["stages"]:
            if (stage.get("moves") or stage.get("expected_sfen", case["sfen"]) != case["sfen"]
                    or len(stage.get("candidates", [])) != 1):
                parser.error("policy coverage requires one direct baseline move per stage")
            tasks.append((case["id"], stage["id"], case["sfen"], stage["candidates"][0]))
    if len(tasks) > args.max_invocations:
        parser.error(f"batch needs {len(tasks)} invocations")
    deadline = time.monotonic() + args.max_total_seconds
    rows = []
    failure = None
    for case_id, stage_id, sfen, target in tasks:
        try:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("policy batch deadline exhausted")
            command = [str(args.probe_bin), "--sfen", sfen, "--target", target,
                       "--top-n", str(args.top_n)]
            result = parse_probe(run_process(command, timeout=remaining).stdout,
                                 target, args.top_n)
            rows.append({"case_id": case_id, "stage_id": stage_id, **result})
        except (subprocess.TimeoutExpired, TimeoutError, RuntimeError, ValueError) as error:
            failure = {"case_id": case_id, "stage_id": stage_id, "target": target,
                       "reason": type(error).__name__, "detail": str(error)[:500]}
            break
    status = "complete" if failure is None else "interrupted"
    hits = sum(row["target_in_top_n"] for row in rows)
    report = {
        "schema": "sekirei.spec-policy-coverage.v1",
        "diagnostic_only": True, "strength_claim": False,
        "scope": "empty_tt_static_legal_top_n_vs_observed_baseline_move",
        "status": status, "planned_invocations": len(tasks),
        "completed_invocations": len(rows), "failure": failure,
        "hits_in_completed_rows": hits,
        "coverage_fraction": [hits, len(rows)] if status == "complete" else None,
        "inputs": {"cases": str(args.cases), "cases_sha256": sha256(args.cases),
                   "probe_bin": str(args.probe_bin),
                   "probe_bin_sha256": sha256(args.probe_bin),
                   "top_n": args.top_n,
                   "max_invocations": args.max_invocations,
                   "max_total_seconds": args.max_total_seconds},
        "rows": rows,
    }
    atomic_json(args.output, report)
    print(json.dumps({"rows": len(rows), "hits": hits, "status": status,
                      "output": str(args.output)}, sort_keys=True))
    return 0 if status == "complete" else 1


if __name__ == "__main__":
    raise SystemExit(main())
