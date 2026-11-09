#!/usr/bin/env python3
"""Check bounded mate after the reply in two hash-bound quiet-ablation cases."""

from __future__ import annotations

import argparse
import json
import subprocess
import time
from pathlib import Path

from run_strategy_mate_proofs import parse_proof_fields
from run_tactical_line_diagnostic import atomic_json, load_cases, run_process, sha256


def selected_ablation(path: Path, cases_hash: str, case_id: str) -> dict:
    report = json.loads(path.read_text(encoding="utf-8"))
    inputs = report.get("inputs", {})
    rows = report.get("rows", [])
    if (report.get("schema") != "sekirei.v2-quiet-trace-ablation.v1"
            or report.get("status") != "complete" or report.get("diagnostic_only") is not True
            or report.get("strength_claim") is not False
            or inputs.get("case_id") != case_id or inputs.get("cases_sha256") != cases_hash
            or inputs.get("max_depth") != 10 or len(rows) != 2
            or [row.get("mode") for row in rows] != ["baseline", "target_exempt"]):
        raise ValueError("ablation report is not the completed equal-depth source")
    root = inputs["root_move"]
    reply = inputs["reply"]
    target_hashes = {row["result"]["target_hash"] for row in rows}
    if (len(target_hashes) != 1
            or any(row["result"]["bestmove"] != root
                   or row["result"]["pv_usi"].split(",")[:2] != [root, reply]
                   or row["result"]["pv_legal"] is not True
                   or row["result"]["search_board_restored"] is not True
                   or row["result"]["aborted"] is not False
                   for row in rows)):
        raise ValueError("ablation rows disagree on the legal root/reply boundary")
    return {"root": root, "reply": reply, "target_hash": target_hashes.pop()}


def parse_history(stdout: str, expected_hash: str) -> str:
    lines = stdout.splitlines()
    if len(lines) != 1:
        raise ValueError("history helper must emit one line")
    parts = lines[0].split("\t")
    fields = dict(field.split("=", 1) for field in parts)
    if len(parts) != 2 or set(fields) != {"sfen", "hash"} or fields["hash"] != expected_hash:
        raise ValueError("replayed root differs from traced target position")
    return fields["sfen"]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cases", required=True, type=Path)
    parser.add_argument("--case-id", action="append", required=True)
    parser.add_argument("--ablation-report", action="append", required=True, type=Path)
    parser.add_argument("--history-bin", required=True, type=Path)
    parser.add_argument("--proof-bin", required=True, type=Path)
    parser.add_argument("--max-ply", type=int, default=7)
    parser.add_argument("--nodes", type=int, default=10000)
    parser.add_argument("--max-total-seconds", type=int, default=30)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if (len(args.case_id) != 2 or len(set(args.case_id)) != 2
            or len(args.ablation_report) != 2 or args.max_ply not in (1, 3, 5, 7)
            or not 1 <= args.nodes <= 10000 or not 1 <= args.max_total_seconds <= 30
            or not args.history_bin.is_file() or not args.proof_bin.is_file()):
        parser.error("requires two distinct cases, two reports, and bounded proof inputs")
    try:
        cases = load_cases(args.cases)
        if cases.get("diagnostic_only") is not True:
            raise ValueError("source cases must be diagnostic only")
        by_id = {case["id"]: case for case in cases["cases"]}
        if len(by_id) != len(cases["cases"]):
            raise ValueError("duplicate case ID")
        selections = []
        for case_id, path in zip(args.case_id, args.ablation_report):
            case = by_id[case_id]
            selection = selected_ablation(path, sha256(args.cases), case_id)
            if (len(case["stages"]) != 1
                    or selection["root"] not in case["stages"][0].get("candidates", [])):
                raise ValueError("ablation root was not an explicit case candidate")
            selections.append((case_id, case["sfen"], path, selection))
    except (OSError, ValueError, KeyError, TypeError) as error:
        parser.error(str(error))
    deadline = time.monotonic() + args.max_total_seconds
    rows = []
    failure = None
    for case_id, sfen, path, selected in selections:
        try:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("batch deadline exhausted")
            history = run_process([str(args.history_bin), "--sfen", sfen,
                                   "--moves", selected["root"]], timeout=remaining)
            child_sfen = parse_history(history.stdout, selected["target_hash"])
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("batch deadline exhausted")
            proof = run_process([str(args.proof_bin), "--sfen", child_sfen,
                                 "--move", selected["reply"],
                                 "--max-ply", str(args.max_ply),
                                 "--nodes", str(args.nodes)], timeout=remaining)
            fields = parse_proof_fields(proof.stdout)
            if (fields["move"] != selected["reply"]
                    or fields["initial_hash"] != selected["target_hash"]
                    or fields["max_ply"] != str(args.max_ply)
                    or fields["node_limit"] != str(args.nodes)
                    or fields["outcome"] not in ("mate", "no_mate", "unknown")
                    or fields["aborted"] not in ("true", "false")
                    or fields["aborted"] == "false" and fields["abort_reason"] != "none"
                    or fields["outcome"] == "no_mate" and
                       (fields["aborted"] != "false" or fields["mate_ply"] != "0")
                    or fields["outcome"] == "mate" and
                       (not 1 <= int(fields["mate_ply"]) <= args.max_ply
                        or not fields["solutions"])
                    or not 0 <= int(fields["nodes"]) <= args.nodes):
                raise ValueError("mate proof violated requested boundary")
            rows.append({"case_id": case_id, "source_report_sha256": sha256(path),
                         "root_move": selected["root"], "reply": selected["reply"],
                         "root_child_hash": selected["target_hash"],
                         "reply_child_hash": fields["child_hash"],
                         "outcome": fields["outcome"],
                         "aborted": fields["aborted"] == "true",
                         "abort_reason": fields["abort_reason"],
                         "invalid_reason": fields["invalid_reason"],
                         "mate_ply": int(fields["mate_ply"]),
                         "solutions": [move for move in fields["solutions"].split(",") if move],
                         "nodes": int(fields["nodes"]),
                         "distance_rule": fields["distance_rule"]})
        except (subprocess.TimeoutExpired, TimeoutError, RuntimeError, ValueError, KeyError,
                OSError) as error:
            failure = {"case_id": case_id, "reason": type(error).__name__,
                       "detail": str(error)[:500]}
            break
    status = "complete" if failure is None else "interrupted"
    report = {"schema": "sekirei.post-root-bounded-mate-check.v1",
              "diagnostic_only": True, "strength_claim": False,
              "scope": "attacker_checking_mate_after_traced_reply_within_max_ply",
              "status": status, "failure": failure,
              "planned_cases": len(selections), "completed_cases": len(rows),
              "inputs": {"cases": str(args.cases), "cases_sha256": sha256(args.cases),
                         "ablation_reports": [{"path": str(path), "sha256": sha256(path)}
                                              for path in args.ablation_report],
                         "history_bin": str(args.history_bin),
                         "history_bin_sha256": sha256(args.history_bin),
                         "proof_bin": str(args.proof_bin),
                         "proof_bin_sha256": sha256(args.proof_bin),
                         "runner_sha256": sha256(Path(__file__).resolve()),
                         "max_ply": args.max_ply, "nodes_per_case": args.nodes,
                         "max_total_seconds": args.max_total_seconds},
              "rows": rows}
    atomic_json(args.output, report)
    print(json.dumps({"status": status, "rows": len(rows),
                      "output": str(args.output)}, sort_keys=True))
    return 0 if status == "complete" else 1


if __name__ == "__main__":
    raise SystemExit(main())
