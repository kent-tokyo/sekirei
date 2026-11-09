#!/usr/bin/env python3
"""Observe actual speculative candidate selection in at most three small cases."""

from __future__ import annotations

import argparse
import json
import subprocess
import time
from pathlib import Path

from run_tactical_line_diagnostic import atomic_json, load_cases, run_process, sha256


def parse_observation(stdout: str, requested_nodes: int, heuristics_mode: str = "fresh") -> dict:
    lines = stdout.splitlines()
    if len(lines) != 1:
        raise ValueError("selection probe must emit exactly one line")
    fields = dict(item.split("=", 1) for item in lines[0].split("\t"))
    expected = {"initial_hash", "bestmove", "bestmove_legal", "pv_usi",
                "pv_legal", "pv_root_matches_bestmove", "completed_depth", "nodes",
                "requested_nodes", "configured_workers", "selected_groups",
                "heuristics_mode", "total_groups", "selected_candidates", "admitted_tasks",
                "cap_rejected", "started_tasks", "completed_tasks", "aborted_tasks",
                "peak_in_flight", "background_nodes", "outstanding_after_join",
                "permit_hold_ns_sum", "permit_hold_ns_max", "queue_wait_ns_sum",
                "queue_wait_ns_max", "started_hold_ns_sum", "started_hold_ns_max",
                "unstarted_hold_ns_sum",
                "child_preflight_ns_sum", "child_legal_check_ns_sum",
                "child_state_setup_ns_sum", "child_kernel_ns_sum",
                "tt_attribution_available", "tt_score_hits_from_speculation",
                "tt_move_hits_from_speculation", "tt_main_cutoffs_from_speculation",
                "tt_nonroot_move_first_started",
                "selection_trace", "board_restored", "diagnostic_only"}
    if set(fields) != expected or fields["requested_nodes"] != str(requested_nodes):
        raise ValueError("selection probe returned mismatched fields")
    if (fields["diagnostic_only"] != "true" or fields["board_restored"] != "true"
            or fields["bestmove_legal"] != "true" or fields["pv_legal"] != "true"
            or fields["pv_root_matches_bestmove"] != "true"
            or fields["heuristics_mode"] != heuristics_mode
            or fields["configured_workers"] != "3"):
        raise ValueError("selection probe violated bounded search contract")
    if fields["tt_attribution_available"] != "true":
        raise ValueError("selection probe violated bounded search contract")
    if len(fields["initial_hash"]) != 16 or any(
        char not in "0123456789abcdef" for char in fields["initial_hash"]
    ):
        raise ValueError("selection probe returned invalid hash")
    groups = []
    for entry in filter(None, fields["selection_trace"].split(";")):
        parts = entry.split(":")
        if len(parts) != 8:
            raise ValueError("selection trace entry has wrong shape")
        (depth, tt, selected_text, selected_count, admitted_text, admitted_count,
         in_flight_before, running_before) = parts
        selected = list(filter(None, selected_text.split(",")))
        admitted = list(filter(None, admitted_text.split(",")))
        selected_count, admitted_count = int(selected_count), int(admitted_count)
        if (not 1 <= int(depth) <= 12 or not 0 <= selected_count <= 2
                or not 0 <= admitted_count <= selected_count
                or len(selected) != selected_count or len(admitted) != admitted_count
                or admitted != selected[:admitted_count]
                or len(selected) != len(set(selected))
                or not 0 <= int(in_flight_before) <= 2
                or not 0 <= int(running_before) <= 2):
            raise ValueError("selection trace violates candidate or admission contract")
        groups.append({"iteration_depth": int(depth), "tt_move": None if tt == "none" else tt,
                       "selected": selected, "admitted": admitted,
                       "in_flight_before": int(in_flight_before),
                       "running_before": int(running_before)})
    selected_groups = int(fields["selected_groups"])
    total_groups = int(fields["total_groups"])
    if (selected_groups != len(groups) or not selected_groups <= total_groups <= 12
            or [group["iteration_depth"] for group in groups] != list(range(1, selected_groups + 1))
            or sum(len(group["selected"]) for group in groups) != int(fields["selected_candidates"])
            or sum(len(group["admitted"]) for group in groups) != int(fields["admitted_tasks"])
            or int(fields["cap_rejected"]) != int(fields["selected_candidates"]) - int(fields["admitted_tasks"])
            or int(fields["started_tasks"]) > int(fields["admitted_tasks"])
            or int(fields["completed_tasks"]) + int(fields["aborted_tasks"]) != int(fields["admitted_tasks"])
            or not 0 <= int(fields["peak_in_flight"]) <= 2
            or int(fields["permit_hold_ns_sum"]) != (
                int(fields["queue_wait_ns_sum"]) + int(fields["started_hold_ns_sum"])
                + int(fields["unstarted_hold_ns_sum"]))
            or any(int(fields[name]) > int(fields[total]) for name, total in (
                ("permit_hold_ns_max", "permit_hold_ns_sum"),
                ("queue_wait_ns_max", "queue_wait_ns_sum"),
                ("started_hold_ns_max", "started_hold_ns_sum")))
            or int(fields["child_preflight_ns_sum"]) + int(fields["child_kernel_ns_sum"]) > int(fields["started_hold_ns_sum"])
            or int(fields["child_legal_check_ns_sum"]) > int(fields["child_preflight_ns_sum"])
            or int(fields["child_state_setup_ns_sum"]) > int(fields["child_preflight_ns_sum"])
            or int(fields["child_legal_check_ns_sum"]) + int(fields["child_state_setup_ns_sum"]) > int(fields["child_preflight_ns_sum"])
            or not 0 <= int(fields["tt_main_cutoffs_from_speculation"]) <= int(fields["tt_score_hits_from_speculation"])
            or (fields["heuristics_mode"] == "background_isolated_tt"
                and any(int(fields[name]) != 0 for name in (
                "tt_score_hits_from_speculation", "tt_move_hits_from_speculation",
                "tt_main_cutoffs_from_speculation", "tt_nonroot_move_first_started")))
            or int(fields["outstanding_after_join"]) != 0):
        raise ValueError("selection trace totals disagree")
    pv = [move for move in fields["pv_usi"].split(",") if move]
    if int(fields["completed_depth"]) > 0 and (not pv or pv[0] != fields["bestmove"]):
        raise ValueError("completed search has no matching PV root")
    return {"initial_hash": fields["initial_hash"], "bestmove": fields["bestmove"],
            "pv_usi": pv, "pv_legal": True, "bestmove_legal": True,
            "heuristics_mode": fields["heuristics_mode"],
            "completed_depth": int(fields["completed_depth"]), "nodes": int(fields["nodes"]),
            "configured_workers": 3, "total_groups": total_groups,
            "cap_rejected": int(fields["cap_rejected"]),
            "started_tasks": int(fields["started_tasks"]),
            "completed_tasks": int(fields["completed_tasks"]),
            "aborted_tasks": int(fields["aborted_tasks"]),
            "peak_in_flight": int(fields["peak_in_flight"]),
            "background_nodes": int(fields["background_nodes"]),
            "tt_attribution_available": True,
            "tt_score_hits_from_speculation": int(fields["tt_score_hits_from_speculation"]),
            "tt_move_hits_from_speculation": int(fields["tt_move_hits_from_speculation"]),
            "tt_main_cutoffs_from_speculation": int(fields["tt_main_cutoffs_from_speculation"]),
            "tt_nonroot_move_first_started": int(fields["tt_nonroot_move_first_started"]),
            "permit_hold_ns_sum": int(fields["permit_hold_ns_sum"]),
            "permit_hold_ns_max": int(fields["permit_hold_ns_max"]),
            "queue_wait_ns_sum": int(fields["queue_wait_ns_sum"]),
            "queue_wait_ns_max": int(fields["queue_wait_ns_max"]),
            "started_hold_ns_sum": int(fields["started_hold_ns_sum"]),
            "started_hold_ns_max": int(fields["started_hold_ns_max"]),
            "unstarted_hold_ns_sum": int(fields["unstarted_hold_ns_sum"]),
            "child_preflight_ns_sum": int(fields["child_preflight_ns_sum"]),
            "child_legal_check_ns_sum": int(fields["child_legal_check_ns_sum"]),
            "child_state_setup_ns_sum": int(fields["child_state_setup_ns_sum"]),
            "child_kernel_ns_sum": int(fields["child_kernel_ns_sum"]),
            "outstanding_after_join": 0, "groups": groups}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cases", required=True, type=Path)
    parser.add_argument("--probe-bin", required=True, type=Path)
    parser.add_argument("--case-id", action="append", required=True)
    parser.add_argument("--nodes", type=int, default=5000)
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument("--shared-heuristics", action="store_true")
    modes.add_argument("--background-shared-heuristics", action="store_true")
    modes.add_argument("--isolated-background-tt", action="store_true")
    parser.add_argument("--max-total-seconds", type=int, default=30)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if (not args.probe_bin.is_file() or not 1 <= len(args.case_id) <= 3
            or len(set(args.case_id)) != len(args.case_id)
            or not 1 <= args.nodes <= 10000 or not 1 <= args.max_total_seconds <= 30):
        parser.error("invalid binary or bounded diagnostic limits")
    cases = load_cases(args.cases)
    def valid_hash(value):
        return isinstance(value, str) and len(value) == 64 and all(
            char in "0123456789abcdef" for char in value)
    baseline_bound = (cases.get("strength_claim") is False and all(
        valid_hash(cases.get(key)) for key in ("source_cases_sha256", "source_report_sha256")))
    confirmation_bound = (cases.get("role") == "confirmation"
                          and cases.get("strength_claim") in (None, False)
                          and valid_hash(cases.get("source_manifest_sha256")))
    if (cases.get("diagnostic_only") is not True
            or (not confirmation_bound if cases.get("role") == "confirmation"
                else not baseline_bound)):
        parser.error("input must be a hash-bound diagnostic or confirmation case file")
    by_id = {case["id"]: case for case in cases["cases"]}
    if len(by_id) != len(cases["cases"]) or any(case_id not in by_id for case_id in args.case_id):
        parser.error("unknown or duplicate case ID")
    for case_id in args.case_id:
        case = by_id[case_id]
        if (len(case["stages"]) != 1 or case["stages"][0].get("moves")
                or case["stages"][0].get("expected_sfen") != case["sfen"]):
            parser.error("case is not bound to the direct root position")
        if (confirmation_bound
                and case.get("source", {}).get("manifest_sha256") != cases["source_manifest_sha256"]):
            parser.error("confirmation case does not match source manifest")
    deadline = time.monotonic() + args.max_total_seconds
    rows = []
    failure = None
    heuristics_mode = ("shared" if args.shared_heuristics else
                       "background_shared" if args.background_shared_heuristics else
                       "background_isolated_tt" if args.isolated_background_tt else "fresh")
    for case_id in args.case_id:
        case = by_id[case_id]
        try:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("batch deadline exhausted")
            command = [str(args.probe_bin), "--sfen", case["sfen"],
                       "--nodes", str(args.nodes), "--max-depth", "12"]
            if args.shared_heuristics:
                command.append("--shared-heuristics")
            elif args.background_shared_heuristics:
                command.append("--background-shared-heuristics")
            elif args.isolated_background_tt:
                command.append("--isolated-background-tt")
            observation = parse_observation(run_process(command, timeout=remaining).stdout,
                                            args.nodes, heuristics_mode)
            rows.append({"case_id": case_id, "sfen": case["sfen"], **observation})
        except (subprocess.TimeoutExpired, TimeoutError, RuntimeError, ValueError) as error:
            failure = {"case_id": case_id, "reason": type(error).__name__,
                       "detail": str(error)[:500]}
            break
    status = "complete" if failure is None else "interrupted"
    report = {"schema": "sekirei.spec-selection-observation.v1",
              "diagnostic_only": True, "strength_claim": False,
              "scope": "actual_spawn_boundary_tt_move_selected_admitted_and_child_setup",
              "status": status, "failure": failure, "planned_cases": len(args.case_id),
              "completed_cases": len(rows),
              "input_role": "confirmation" if confirmation_bound else "development_baseline",
              "inputs": {"cases": str(args.cases), "cases_sha256": sha256(args.cases),
                         "probe_bin": str(args.probe_bin),
                         "probe_bin_sha256": sha256(args.probe_bin),
                         "nodes": args.nodes, "heuristics_mode": heuristics_mode,
                         "max_total_seconds": args.max_total_seconds},
              "rows": rows}
    atomic_json(args.output, report)
    print(json.dumps({"status": status, "rows": len(rows),
                      "output": str(args.output)}, sort_keys=True))
    return 0 if status == "complete" else 1


if __name__ == "__main__":
    raise SystemExit(main())
