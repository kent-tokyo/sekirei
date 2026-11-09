#!/usr/bin/env python3
"""Check bounded checking mate after the two hash-bound guarded recaptures."""

from __future__ import annotations

import argparse
import json
import subprocess
import time
from pathlib import Path

from run_post_root_mate_check import parse_history
from run_strategy_mate_proofs import parse_proof_fields
from run_tactical_line_diagnostic import atomic_json, load_cases, run_process, sha256


def select_recaptures(ledger: dict, cases: dict, cases_hash: str,
                      spec_hash: str) -> list[dict]:
    inputs = ledger.get("inputs", {})
    if (ledger.get("schema") != "sekirei.strategy-reply-mobility-report.v4"
            or ledger.get("status") != "complete"
            or ledger.get("diagnostic_only") is not True
            or ledger.get("strength_claim") is not False
            or inputs.get("cases_sha256") != cases_hash
            or inputs.get("spec_sha256") != spec_hash
            or inputs.get("material_ledger") is not True
            or ledger.get("completed_rows") != 2 or len(ledger.get("rows", [])) != 2):
        raise ValueError("ledger is not the completed guarded v4 source")
    by_id = {case["id"]: case for case in cases["cases"]}
    if len(by_id) != len(cases["cases"]):
        raise ValueError("duplicate source case ID")
    focus = by_id["pinned-guarded-focus"]
    row = next((row for row in ledger["rows"]
                if row.get("case_id") == "pinned-guarded-focus"), None)
    if (row is None or {row.get("case_id") for row in ledger["rows"]}
            != {"pinned-guarded-focus", "pinned-guarded-contrast"}
            or row.get("candidate") != "5h4h" or row.get("origin") != "5g"
            or row.get("status") != "complete"
            or focus["stages"][0].get("candidates") != ["5h4h"]
            or row.get("reply_count") != len(row.get("reply_rows", []))):
        raise ValueError("guarded focus does not match source case")
    selected = []
    for reply in row["reply_rows"]:
        if reply.get("reply") not in ("5c5g", "5c5g+"):
            continue
        board_hash = reply.get("reply_hash")
        if (reply.get("piece_present") is not False
                or not isinstance(reply.get("in_check"), bool)
                or reply.get("legal_recapture_count") != 1
                or [item.get("move") for item in reply.get("recapture_material", [])]
                   != ["4h5g"]
                or not isinstance(board_hash, str) or len(board_hash) != 16
                or any(char not in "0123456789abcdef" for char in board_hash)):
            raise ValueError("capture row has no verified legal king recapture")
        selected.append({"case_id": focus["id"], "sfen": focus["sfen"],
                         "candidate": row["candidate"], "reply": reply["reply"],
                         "reply_hash": board_hash, "recapture": "4h5g"})
    if len(selected) != 2 or {item["reply"] for item in selected} != {"5c5g", "5c5g+"}:
        raise ValueError("both guarded rook captures must be present")
    return selected


def validate_proof(fields: dict, selection: dict, max_ply: int, nodes: int) -> dict:
    outcome = fields["outcome"]
    aborted = fields["aborted"]
    mate_ply = int(fields["mate_ply"])
    used_nodes = int(fields["nodes"])
    if (fields["move"] != selection["recapture"]
            or fields["initial_hash"] != selection["reply_hash"]
            or fields["max_ply"] != str(max_ply)
            or fields["node_limit"] != str(nodes)
            or outcome not in ("mate", "no_mate", "unknown")
            or aborted not in ("true", "false")
            or (aborted == "false" and fields["abort_reason"] != "none")
            or not 0 <= used_nodes <= nodes
            or (outcome == "no_mate" and (aborted != "false" or mate_ply != 0))
            or (outcome == "mate" and (not 1 <= mate_ply <= max_ply
                                       or not fields["solutions"]))):
        raise ValueError("mate proof violated requested boundary")
    return {"case_id": selection["case_id"], "candidate": selection["candidate"],
            "reply": selection["reply"], "reply_hash": selection["reply_hash"],
            "recapture": selection["recapture"], "recapture_child_hash": fields["child_hash"],
            "outcome": outcome, "aborted": aborted == "true",
            "abort_reason": fields["abort_reason"], "invalid_reason": fields["invalid_reason"],
            "mate_ply": mate_ply,
            "solutions": [move for move in fields["solutions"].split(",") if move],
            "nodes": used_nodes, "distance_rule": fields["distance_rule"]}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cases", required=True, type=Path)
    parser.add_argument("--spec", required=True, type=Path)
    parser.add_argument("--ledger", required=True, type=Path)
    parser.add_argument("--history-bin", required=True, type=Path)
    parser.add_argument("--proof-bin", required=True, type=Path)
    parser.add_argument("--max-ply", type=int, default=7)
    parser.add_argument("--nodes", type=int, default=10000)
    parser.add_argument("--max-total-seconds", type=int, default=30)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if (args.max_ply not in (1, 3, 5, 7) or not 1 <= args.nodes <= 10000
            or not 1 <= args.max_total_seconds <= 30
            or not args.history_bin.is_file() or not args.proof_bin.is_file()):
        parser.error("proof inputs must exist and use bounded limits")
    try:
        cases = load_cases(args.cases)
        spec = json.loads(args.spec.read_text(encoding="utf-8"))
        if (cases.get("diagnostic_only") is not True
                or spec.get("selections") != [
                    {"case_id": "pinned-guarded-focus", "candidate": "5h4h", "origin": "5g"},
                    {"case_id": "pinned-guarded-contrast", "candidate": "5h4h", "origin": "5g"}]):
            raise ValueError("input is not the guarded focus and contrast pair")
        ledger = json.loads(args.ledger.read_text(encoding="utf-8"))
        selected = select_recaptures(ledger, cases, sha256(args.cases), sha256(args.spec))
    except (OSError, ValueError, KeyError, TypeError) as error:
        parser.error(str(error))
    deadline = time.monotonic() + args.max_total_seconds
    rows = []
    failure = None
    for item in selected:
        try:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("batch deadline exhausted")
            replay = run_process([str(args.history_bin), "--sfen", item["sfen"],
                                  "--moves", f"{item['candidate']} {item['reply']}"],
                                 timeout=remaining)
            reply_sfen = parse_history(replay.stdout, item["reply_hash"])
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("batch deadline exhausted")
            proof = run_process([str(args.proof_bin), "--sfen", reply_sfen,
                                 "--move", item["recapture"],
                                 "--max-ply", str(args.max_ply), "--nodes", str(args.nodes)],
                                timeout=remaining)
            rows.append(validate_proof(parse_proof_fields(proof.stdout), item,
                                       args.max_ply, args.nodes))
        except (subprocess.TimeoutExpired, TimeoutError, RuntimeError, ValueError,
                KeyError, OSError) as error:
            failure = {"reply": item["reply"], "reason": type(error).__name__,
                       "detail": str(error)[:500]}
            break
    report = {"schema": "sekirei.guarded-recapture-bounded-mate.v1",
              "diagnostic_only": True, "strength_claim": False,
              "scope": "opponent_checking_mate_after_king_recapture_within_max_ply",
              "status": "complete" if failure is None else "interrupted",
              "planned_rows": len(selected), "completed_rows": len(rows), "failure": failure,
              "inputs": {"cases": str(args.cases), "cases_sha256": sha256(args.cases),
                         "spec": str(args.spec), "spec_sha256": sha256(args.spec),
                         "ledger": str(args.ledger), "ledger_sha256": sha256(args.ledger),
                         "history_bin": str(args.history_bin),
                         "history_bin_sha256": sha256(args.history_bin),
                         "proof_bin": str(args.proof_bin),
                         "proof_bin_sha256": sha256(args.proof_bin),
                         "runner_sha256": sha256(Path(__file__).resolve()),
                         "max_ply": args.max_ply, "nodes_per_row": args.nodes,
                         "max_total_seconds": args.max_total_seconds},
              "rows": rows}
    atomic_json(args.output, report)
    print(json.dumps({"status": report["status"], "rows": len(rows),
                      "output": str(args.output)}, sort_keys=True))
    return 0 if failure is None else 1


if __name__ == "__main__":
    raise SystemExit(main())
