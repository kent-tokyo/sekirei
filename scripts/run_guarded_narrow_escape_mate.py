#!/usr/bin/env python3
"""Check bounded mate after both legal escapes from guarded `3c3h+`."""

from __future__ import annotations

import argparse
import json
import subprocess
import time
from pathlib import Path

from run_guarded_check_evasions import parse_legal_moves
from run_post_root_mate_check import parse_history
from run_strategy_mate_proofs import parse_proof_fields
from run_tactical_line_diagnostic import atomic_json, load_cases, run_process, sha256


def select_narrow_check(report: dict, ledger: dict, cases_hash: str,
                        ledger_hash: str) -> dict:
    inputs = report.get("inputs", {})
    rows = report.get("rows", [])
    if (report.get("schema") != "sekirei.guarded-check-evasions.v1"
            or report.get("status") != "complete"
            or report.get("diagnostic_only") is not True
            or report.get("strength_claim") is not False
            or report.get("planned_rows") != 7 or report.get("completed_rows") != 7
            or len(rows) != 7 or inputs.get("cases_sha256") != cases_hash
            or inputs.get("ledger_sha256") != ledger_hash):
        raise ValueError("evasion report is not the completed guarded source")
    target = [row for row in rows if row.get("case_id") == "pinned-guarded-contrast"
              and row.get("reply") == "3c3h+"]
    if len(target) != 1:
        raise ValueError("narrow checking reply is missing or duplicated")
    target = target[0]
    source = next((row for row in ledger.get("rows", [])
                   if row.get("case_id") == "pinned-guarded-contrast"), None)
    source_reply = next((row for row in source.get("reply_rows", [])
                         if row.get("reply") == "3c3h+"), None) if source else None
    if (ledger.get("schema") != "sekirei.strategy-reply-mobility-report.v4"
            or ledger.get("status") != "complete"
            or source_reply is None or source_reply.get("in_check") is not True
            or source_reply.get("reply_hash") != target.get("reply_hash")
            or target.get("candidate") != "5h4h"
            or target.get("checkmate_now") is not False
            or target.get("legal_evasions") != ["4h3h", "4h5i"]
            or target.get("king_evasions") != ["4h3h", "4h5i"]):
        raise ValueError("narrow check is not bound to both king escapes")
    return target


def validate_proof(fields: dict, expected_hash: str, move: str,
                   max_ply: int, nodes: int) -> dict:
    outcome = fields["outcome"]
    aborted = fields["aborted"]
    mate_ply = int(fields["mate_ply"])
    used_nodes = int(fields["nodes"])
    if (fields["initial_hash"] != expected_hash or fields["move"] != move
            or fields["max_ply"] != str(max_ply)
            or fields["node_limit"] != str(nodes)
            or outcome not in ("mate", "no_mate", "unknown")
            or aborted not in ("true", "false")
            or aborted == "false" and fields["abort_reason"] != "none"
            or outcome == "no_mate" and (aborted != "false" or mate_ply != 0)
            or outcome == "mate" and (not 1 <= mate_ply <= max_ply
                                       or not fields["solutions"])
            or not 0 <= used_nodes <= nodes):
        raise ValueError("mate proof violated requested boundary")
    return {"escape": move, "reply_hash": expected_hash,
            "escape_child_hash": fields["child_hash"], "outcome": outcome,
            "aborted": aborted == "true", "abort_reason": fields["abort_reason"],
            "invalid_reason": fields["invalid_reason"], "mate_ply": mate_ply,
            "solutions": [move for move in fields["solutions"].split(",") if move],
            "nodes": used_nodes, "distance_rule": fields["distance_rule"]}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cases", type=Path, required=True)
    parser.add_argument("--ledger", type=Path, required=True)
    parser.add_argument("--evasion-report", type=Path, required=True)
    parser.add_argument("--history-bin", type=Path, required=True)
    parser.add_argument("--legal-bin", type=Path, required=True)
    parser.add_argument("--proof-bin", type=Path, required=True)
    parser.add_argument("--max-ply", type=int, default=7)
    parser.add_argument("--nodes", type=int, default=10000)
    parser.add_argument("--max-total-seconds", type=int, default=20)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if (args.max_ply not in (1, 3, 5, 7) or not 1 <= args.nodes <= 10000
            or not 1 <= args.max_total_seconds <= 30
            or not all(path.is_file() for path in
                       (args.history_bin, args.legal_bin, args.proof_bin))):
        parser.error("requires existing helpers and bounded proof limits")
    try:
        cases = load_cases(args.cases)
        case = next(case for case in cases["cases"]
                    if case["id"] == "pinned-guarded-contrast")
        ledger = json.loads(args.ledger.read_text(encoding="utf-8"))
        report = json.loads(args.evasion_report.read_text(encoding="utf-8"))
        selected = select_narrow_check(report, ledger, sha256(args.cases),
                                       sha256(args.ledger))
    except (OSError, ValueError, KeyError, TypeError, StopIteration) as error:
        parser.error(str(error))
    deadline = time.monotonic() + args.max_total_seconds
    rows = []
    failure = None
    try:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("batch deadline exhausted")
        replay = run_process([str(args.history_bin), "--sfen", case["sfen"],
                              "--moves", "5h4h 3c3h+"], timeout=remaining)
        reply_sfen = parse_history(replay.stdout, selected["reply_hash"])
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("batch deadline exhausted")
        legal = subprocess.run([str(args.legal_bin)], input=reply_sfen + "\n",
                               text=True, capture_output=True, check=False,
                               timeout=remaining)
        if legal.returncode or parse_legal_moves(legal.stdout) != selected["legal_evasions"]:
            raise ValueError("current legal evasions differ from source report")
        for move in selected["legal_evasions"]:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("batch deadline exhausted")
            proof = run_process([str(args.proof_bin), "--sfen", reply_sfen,
                                 "--move", move, "--max-ply", str(args.max_ply),
                                 "--nodes", str(args.nodes)], timeout=remaining)
            rows.append(validate_proof(parse_proof_fields(proof.stdout),
                                       selected["reply_hash"], move,
                                       args.max_ply, args.nodes))
    except (subprocess.TimeoutExpired, TimeoutError, RuntimeError, ValueError,
            KeyError, OSError) as error:
        failure = {"escape": selected["legal_evasions"][len(rows)] if len(rows) < 2 else None,
                   "reason": type(error).__name__, "detail": str(error)[:500]}
    result = {"schema": "sekirei.guarded-narrow-escape-bounded-mate.v1",
              "diagnostic_only": True, "strength_claim": False,
              "scope": "opponent_checking_mate_after_each_legal_escape_from_3c3h_plus",
              "status": "complete" if failure is None else "interrupted",
              "planned_rows": 2, "completed_rows": len(rows), "failure": failure,
              "case_id": "pinned-guarded-contrast", "candidate": "5h4h",
              "reply": "3c3h+", "reply_hash": selected["reply_hash"],
              "inputs": {"cases": str(args.cases), "cases_sha256": sha256(args.cases),
                         "ledger": str(args.ledger), "ledger_sha256": sha256(args.ledger),
                         "evasion_report": str(args.evasion_report),
                         "evasion_report_sha256": sha256(args.evasion_report),
                         "history_bin_sha256": sha256(args.history_bin),
                         "legal_bin_sha256": sha256(args.legal_bin),
                         "proof_bin_sha256": sha256(args.proof_bin),
                         "runner_sha256": sha256(Path(__file__).resolve()),
                         "max_ply": args.max_ply, "nodes_per_escape": args.nodes,
                         "max_total_seconds": args.max_total_seconds},
              "rows": rows}
    atomic_json(args.output, result)
    print(json.dumps({"status": result["status"], "rows": len(rows),
                      "output": str(args.output)}, sort_keys=True))
    return 0 if failure is None else 1


if __name__ == "__main__":
    raise SystemExit(main())
