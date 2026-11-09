#!/usr/bin/env python3
"""Compare empty-TT policy with a synthetic prior-completed-root TT hint.

Every search is a separate single-thread, material-only, bounded diagnostic.
The seeded TT is not a live speculative table; no task or useful-work claim
follows from this comparison.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import time
from pathlib import Path

from run_spec_policy_coverage import parse_probe
from run_tactical_line_diagnostic import atomic_json, load_cases, run_process, sha256


def parse_trace(stdout: str) -> dict:
    lines = stdout.splitlines()
    if len(lines) != 1:
        raise ValueError("search trace must have one output line")
    fields = dict(item.split("=", 1) for item in lines[0].split("\t"))
    if (fields.get("pv_legal") != "true" or fields.get("pv_replay_preserves_input") != "true"
            or fields.get("completed_iteration_valid") != "true"
            or fields.get("completed_bound") != "exact"
            or fields.get("bestmove_matches_pv_root") != "true"
            or fields.get("pv_root") != fields.get("bestmove")
            or fields.get("history_final_hash") != fields.get("history_initial_hash")):
        raise ValueError("search trace has no consistent completed PV")
    iterations = [part.split(":") for part in fields["iteration_trace"].split(",") if part]
    if len(iterations) < 2 or any(len(part) != 7 for part in iterations):
        raise ValueError("search trace requires at least two completed iterations")
    if (iterations[-1][1] != fields["bestmove"]
            or iterations[-1][4] != "exact" or iterations[-2][4] != "exact" or
            iterations[-1][1] == "resign" or iterations[-2][1] == "resign"):
        raise ValueError("completed trace and returned move disagree")
    return {"bestmove": fields["bestmove"], "pv_root": fields["pv_root"],
            "initial_hash": fields["history_initial_hash"],
            "completed_iterations": len(iterations),
            "prior_depth": int(iterations[-2][0]), "prior_move": iterations[-2][1],
            "final_depth": int(iterations[-1][0]), "nodes": int(fields["nodes"])}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cases", required=True, type=Path)
    parser.add_argument("--search-bin", required=True, type=Path)
    parser.add_argument("--probe-bin", required=True, type=Path)
    parser.add_argument("--nodes", type=int, default=10000)
    parser.add_argument("--top-n", type=int, default=2)
    parser.add_argument("--max-cases", type=int, default=12)
    parser.add_argument("--max-total-seconds", type=int, default=90)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if (not args.search_bin.is_file() or not args.probe_bin.is_file()
            or not 1 <= args.nodes <= 10000 or not 1 <= args.top_n <= 16
            or not 1 <= args.max_cases <= 24 or not 1 <= args.max_total_seconds <= 90):
        parser.error("invalid binary or bounded diagnostic limits")
    cases = load_cases(args.cases)
    if (cases.get("diagnostic_only") is not True or cases.get("strength_claim") is not False
            or cases.get("nodes_per_baseline_search") != args.nodes
            or any(not isinstance(cases.get(key), str) or len(cases[key]) != 64
                   for key in ("source_cases_sha256", "source_report_sha256"))):
        parser.error("input must be a hash-bound baseline case file at the same node limit")
    if len(cases["cases"]) > args.max_cases:
        parser.error("case count exceeds --max-cases")
    deadline = time.monotonic() + args.max_total_seconds
    rows = []
    failure = None
    for case in cases["cases"]:
        case_id, sfen = case["id"], case["sfen"]
        try:
            stage, = case["stages"]
            target, = stage["candidates"]
            if stage.get("moves") or stage.get("expected_sfen") != sfen:
                raise ValueError("case is not a direct root baseline")
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("batch deadline exhausted")
            trace = parse_trace(run_process([
                str(args.search_bin), "--sfen", sfen, "--nodes", str(args.nodes),
                "--iteration-trace",
            ], timeout=remaining).stdout)
            row = {"case_id": case_id, "target": target, "trace": trace}
            if trace["bestmove"] != target:
                row["status"] = "baseline_mismatch"
                rows.append(row)
                continue
            comparisons = {}
            for mode, hint in (("empty", None), ("prior_root_hint", trace["prior_move"])):
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError("batch deadline exhausted")
                command = [str(args.probe_bin), "--sfen", sfen, "--target", target,
                           "--top-n", str(args.top_n)]
                if hint:
                    command += ["--tt-hint", hint]
                comparisons[mode] = parse_probe(
                    run_process(command, timeout=remaining).stdout, target, args.top_n, hint)
            if comparisons["empty"]["initial_hash"] != comparisons["prior_root_hint"]["initial_hash"]:
                raise ValueError("policy probes disagree on position hash")
            if comparisons["empty"]["initial_hash"] != trace["initial_hash"]:
                raise ValueError("search trace and policy probes disagree on position hash")
            row.update(status="compared", comparisons=comparisons)
            rows.append(row)
        except (subprocess.TimeoutExpired, TimeoutError, RuntimeError, ValueError) as error:
            failure = {"case_id": case_id, "reason": type(error).__name__,
                       "detail": str(error)[:500]}
            break
    compared = [row for row in rows if row["status"] == "compared"]
    prior_same = sum(row["trace"]["prior_move"] == row["target"] for row in compared)
    prior_different = [row for row in compared if row["trace"]["prior_move"] != row["target"]]
    hits = {mode: sum(row["comparisons"][mode]["target_in_top_n"] for row in compared)
            for mode in ("empty", "prior_root_hint")}
    status = "complete" if failure is None else "interrupted"
    report = {"schema": "sekirei.spec-prior-root-comparison.v1",
              "diagnostic_only": True, "strength_claim": False,
              "scope": "empty_tt_vs_synthetic_prior_completed_root_hint",
              "status": status, "failure": failure, "planned_cases": len(cases["cases"]),
              "completed_cases": len(rows), "compared_cases": len(compared),
              "prior_move_matches_target": prior_same,
              "prior_move_differs_from_target": len(prior_different),
              "different_prior_move_hits": {
                  mode: [sum(row["comparisons"][mode]["target_in_top_n"]
                             for row in prior_different), len(prior_different)]
                  for mode in ("empty", "prior_root_hint")
              } if status == "complete" else None,
              "coverage_fraction": {mode: [count, len(compared)] for mode, count in hits.items()}
              if status == "complete" else None,
              "inputs": {"cases": str(args.cases), "cases_sha256": sha256(args.cases),
                         "search_bin": str(args.search_bin),
                         "search_bin_sha256": sha256(args.search_bin),
                         "probe_bin": str(args.probe_bin),
                         "probe_bin_sha256": sha256(args.probe_bin),
                         "nodes": args.nodes, "top_n": args.top_n,
                         "max_cases": args.max_cases,
                         "max_total_seconds": args.max_total_seconds},
              "rows": rows}
    atomic_json(args.output, report)
    print(json.dumps({"status": status, "compared": len(compared), "hits": hits,
                      "output": str(args.output)}, sort_keys=True))
    return 0 if status == "complete" else 1


if __name__ == "__main__":
    raise SystemExit(main())
