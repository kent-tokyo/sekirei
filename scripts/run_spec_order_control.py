#!/usr/bin/env python3
"""Repeat three speculative controls in a balanced order on two small cases."""

from __future__ import annotations

import argparse
import json
import subprocess
import time
from pathlib import Path

from run_spec_selection_observation import parse_observation
from run_tactical_line_diagnostic import atomic_json, load_cases, run_process, sha256


MODES = ("fresh", "background_shared", "background_isolated_tt")
ORDER = MODES + tuple(reversed(MODES))
FLAGS = {"fresh": None, "background_shared": "--background-shared-heuristics",
         "background_isolated_tt": "--isolated-background-tt"}


def validate_confirmation(cases: dict, case_ids: list[str]) -> dict[str, dict]:
    digest = cases.get("source_manifest_sha256")
    if (cases.get("role") != "confirmation" or cases.get("diagnostic_only") is not True
            or cases.get("strength_claim") not in (None, False)
            or not isinstance(digest, str) or len(digest) != 64
            or any(char not in "0123456789abcdef" for char in digest)):
        raise ValueError("requires a hash-bound confirmation case file")
    by_id = {case["id"]: case for case in cases["cases"]}
    if len(by_id) != len(cases["cases"]) or len(case_ids) != 2 or len(set(case_ids)) != 2:
        raise ValueError("requires two distinct case IDs")
    for case_id in case_ids:
        case = by_id.get(case_id)
        if case is None or case.get("source", {}).get("manifest_sha256") != digest:
            raise ValueError("case is absent or has a different source manifest")
        stages = case["stages"]
        if (len(stages) != 1 or stages[0].get("moves")
                or stages[0].get("expected_sfen") != case["sfen"]):
            raise ValueError("case must be bound to its direct root position")
    return by_id


def summarize(rows: list[dict], case_ids: list[str]) -> list[dict]:
    result = []
    for case_id in case_ids:
        case_rows = [row for row in rows if row["case_id"] == case_id]
        if len({row["initial_hash"] for row in case_rows}) != 1:
            raise ValueError("case hash changed between runs")
        for mode in MODES:
            selected = [row for row in case_rows if row["heuristics_mode"] == mode]
            if len(selected) != 2:
                raise ValueError("summary requires two runs per case and mode")
            result.append({"case_id": case_id, "heuristics_mode": mode,
                           "bestmoves": [row["bestmove"] for row in selected],
                           "distinct_bestmoves": len({row["bestmove"] for row in selected}),
                           "completed_depths": [row["completed_depth"] for row in selected],
                           "started_tasks": [row["started_tasks"] for row in selected],
                           "tt_score_hits_from_speculation": [
                               row["tt_score_hits_from_speculation"] for row in selected]})
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cases", required=True, type=Path)
    parser.add_argument("--probe-bin", required=True, type=Path)
    parser.add_argument("--case-id", action="append", required=True)
    parser.add_argument("--nodes", type=int, default=10000)
    parser.add_argument("--max-total-seconds", type=int, default=90)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if (not args.probe_bin.is_file() or not 1 <= args.nodes <= 10000
            or not 1 <= args.max_total_seconds <= 90):
        parser.error("invalid binary or bounded diagnostic limits")
    try:
        by_id = validate_confirmation(load_cases(args.cases), args.case_id)
    except (OSError, ValueError, KeyError, TypeError) as error:
        parser.error(str(error))
    deadline = time.monotonic() + args.max_total_seconds
    rows = []
    failure = None
    initial_hash_by_case = {}
    schedule = [(case_id, mode) for case_id in args.case_id for mode in ORDER]
    for case_id, mode in schedule:
        try:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("batch deadline exhausted")
            command = [str(args.probe_bin), "--sfen", by_id[case_id]["sfen"],
                       "--nodes", str(args.nodes), "--max-depth", "12"]
            if FLAGS[mode]:
                command.append(FLAGS[mode])
            observation = parse_observation(run_process(command, timeout=remaining).stdout,
                                            args.nodes, mode)
            previous_hash = initial_hash_by_case.setdefault(case_id, observation["initial_hash"])
            if observation["initial_hash"] != previous_hash:
                raise ValueError("case hash changed between runs")
            rows.append({"case_id": case_id, "run_index": len(rows) + 1, **observation})
        except (subprocess.TimeoutExpired, TimeoutError, RuntimeError, ValueError) as error:
            failure = {"case_id": case_id, "heuristics_mode": mode,
                       "reason": type(error).__name__, "detail": str(error)[:500]}
            break
    status = "complete" if failure is None else "interrupted"
    report = {"schema": "sekirei.spec-order-control.v1", "diagnostic_only": True,
              "strength_claim": False, "status": status, "failure": failure,
              "planned_runs": len(schedule), "completed_runs": len(rows),
              "schedule": [{"case_id": case_id, "heuristics_mode": mode}
                           for case_id, mode in schedule],
              "inputs": {"cases": str(args.cases), "cases_sha256": sha256(args.cases),
                         "probe_bin": str(args.probe_bin),
                         "probe_bin_sha256": sha256(args.probe_bin), "nodes": args.nodes,
                         "max_total_seconds": args.max_total_seconds},
              "rows": rows,
              "summary": summarize(rows, args.case_id) if status == "complete" else None}
    atomic_json(args.output, report)
    print(json.dumps({"status": status, "runs": len(rows), "output": str(args.output)},
                     sort_keys=True))
    return 0 if status == "complete" else 1


if __name__ == "__main__":
    raise SystemExit(main())
