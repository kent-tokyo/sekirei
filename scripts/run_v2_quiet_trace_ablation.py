#!/usr/bin/env python3
"""Compare a traced quiet reply with and without its local pruning exemption."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import time
from pathlib import Path

from run_fixed_selfplay_diagnostic import parse_fields
from run_tactical_line_diagnostic import atomic_json, load_cases, run_process, sha256
from run_v2_move_trace_diagnostic import parse_result


def select_case(cases: dict, case_id: str, root_move: str) -> dict:
    digest = cases.get("legal_moves_sha256")
    if (cases.get("diagnostic_only") is not True or cases.get("strength_claim") not in (None, False)
            or not isinstance(digest, str) or len(digest) != 64
            or any(char not in "0123456789abcdef" for char in digest)):
        raise ValueError("requires a hash-bound diagnostic case file")
    matches = [case for case in cases["cases"] if case["id"] == case_id]
    if (len(matches) != 1 or len(matches[0]["stages"]) != 1
            or matches[0]["stages"][0].get("moves")
            or matches[0]["stages"][0].get("expected_sfen") != matches[0]["sfen"]
            or root_move not in matches[0]["stages"][0].get("candidates", [])):
        raise ValueError("case must be a unique direct-root position with the requested candidate")
    return matches[0]


def parse_mode(fields: dict[str, str], reply: str, root_move: str, exempt: bool) -> dict:
    if fields.get("v2_quiet_trace_exemption") != str(exempt).lower():
        raise ValueError("probe returned a different exemption mode")
    result = parse_result(fields, reply)
    if result["bestmove"] != root_move or result["pv_usi"].split(",")[0] != root_move:
        raise ValueError("fixed-root result or PV differs from requested root")
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cases", required=True, type=Path)
    parser.add_argument("--case-id", required=True)
    parser.add_argument("--engine", required=True, type=Path)
    parser.add_argument("--root-move", required=True)
    parser.add_argument("--reply", required=True)
    parser.add_argument("--nodes", required=True, type=int)
    parser.add_argument("--max-depth", type=int)
    parser.add_argument("--max-total-seconds", type=int, default=30)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if (not args.engine.is_file() or not 1 <= args.nodes <= 50000
            or args.max_depth is not None and not 1 <= args.max_depth <= 12
            or not 1 <= args.max_total_seconds <= 90):
        parser.error("invalid binary or bounded diagnostic limits")
    try:
        case = select_case(load_cases(args.cases), args.case_id, args.root_move)
    except (OSError, ValueError, KeyError, TypeError) as error:
        parser.error(str(error))
    deadline = time.monotonic() + args.max_total_seconds
    rows = []
    failure = None
    for exempt in (False, True):
        try:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("batch deadline exhausted")
            command = [str(args.engine), "--nodes", str(args.nodes),
                       "--sfen", case["sfen"], "--root-move", args.root_move,
                       "--trace-after-root-move", args.root_move,
                       "--trace-move", args.reply]
            if args.max_depth is not None:
                command.extend(("--max-depth", str(args.max_depth)))
            if exempt:
                command.append("--exempt-traced-quiet-move")
            fields = parse_fields(run_process(
                command, timeout=remaining, env={**os.environ, "RAYON_NUM_THREADS": "1"}
            ).stdout)
            if fields.get("configured_node_limit") != str(args.nodes):
                raise ValueError("explicit node cap was not applied")
            result = parse_mode(fields, args.reply, args.root_move, exempt)
            if fields.get("aborted") not in ("true", "false"):
                raise ValueError("probe returned an invalid abort state")
            if args.max_depth is not None and (result["depth"] != args.max_depth
                                               or fields.get("aborted") != "false"):
                raise ValueError("requested depth was not completed within the node cap")
            result["configured_node_limit"] = int(fields["configured_node_limit"])
            result["aborted"] = fields["aborted"] == "true"
            result["pv_legal"] = True
            result["search_board_restored"] = True
            if rows and result["target_hash"] != rows[0]["result"]["target_hash"]:
                raise ValueError("target position hash changed between modes")
            rows.append({"mode": "target_exempt" if exempt else "baseline",
                         "result": result})
        except (subprocess.TimeoutExpired, TimeoutError, RuntimeError, ValueError, KeyError,
                OSError) as error:
            failure = {"mode": "target_exempt" if exempt else "baseline",
                       "reason": type(error).__name__, "detail": str(error)[:500]}
            break
    status = "complete" if failure is None else "interrupted"
    report = {"schema": "sekirei.v2-quiet-trace-ablation.v1",
              "diagnostic_only": True, "strength_claim": False,
              "status": status, "failure": failure,
              "planned_invocations": 2, "completed_invocations": len(rows),
              "inputs": {"cases": str(args.cases), "cases_sha256": sha256(args.cases),
                         "case_id": args.case_id, "sfen": case["sfen"],
                         "engine": str(args.engine), "engine_sha256": sha256(args.engine),
                         "runner_sha256": sha256(Path(__file__).resolve()),
                         "rayon_threads": 1,
                         "root_move": args.root_move, "reply": args.reply,
                         "nodes_per_process": args.nodes,
                         "max_depth": args.max_depth,
                         "max_total_seconds": args.max_total_seconds},
              "rows": rows}
    atomic_json(args.output, report)
    print(json.dumps({"status": status, "rows": len(rows), "output": str(args.output)},
                     sort_keys=True))
    return 0 if status == "complete" else 1


if __name__ == "__main__":
    raise SystemExit(main())
