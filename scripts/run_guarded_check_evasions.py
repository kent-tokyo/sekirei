#!/usr/bin/env python3
"""Enumerate legal evasions to every hash-bound checking reply in guarded cases."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import time
from pathlib import Path

from run_post_root_mate_check import parse_history
from run_tactical_line_diagnostic import atomic_json, load_cases, run_process, sha256


USI_MOVE = re.compile(r"(?:[1-9][a-i][1-9][a-i]\+?|[PLNSGBR]\*[1-9][a-i])\Z")
CASE_IDS = {"pinned-guarded-focus", "pinned-guarded-contrast"}


def select_checks(ledger: dict, cases: dict, cases_hash: str, spec_hash: str) -> list[dict]:
    inputs = ledger.get("inputs", {})
    rows = ledger.get("rows", [])
    if (ledger.get("schema") != "sekirei.strategy-reply-mobility-report.v4"
            or ledger.get("status") != "complete"
            or ledger.get("diagnostic_only") is not True
            or ledger.get("strength_claim") is not False
            or ledger.get("planned_rows") != 2 or ledger.get("completed_rows") != 2
            or inputs.get("cases_sha256") != cases_hash
            or inputs.get("spec_sha256") != spec_hash
            or inputs.get("material_ledger") is not True
            or len(rows) != 2 or {row.get("case_id") for row in rows} != CASE_IDS):
        raise ValueError("ledger is not the completed guarded v4 source")
    by_id = {case["id"]: case for case in cases["cases"]}
    if set(by_id) != CASE_IDS:
        raise ValueError("source cases differ from guarded pair")
    selected = []
    for row in rows:
        case = by_id[row["case_id"]]
        details = row.get("reply_rows", [])
        if (row.get("status") != "complete" or row.get("candidate") != "5h4h"
                or row.get("origin") != "5g"
                or case["stages"][0].get("candidates") != ["5h4h"]
                or row.get("reply_count") != len(details)
                or row.get("checking_reply_count")
                   != sum(reply.get("in_check") is True for reply in details)
                or len({reply.get("reply") for reply in details}) != len(details)):
            raise ValueError("guarded row disagrees with case or aggregate")
        for reply in details:
            if reply.get("in_check") is not True:
                continue
            board_hash = reply.get("reply_hash")
            if (not isinstance(board_hash, str) or len(board_hash) != 16
                    or any(char not in "0123456789abcdef" for char in board_hash)
                    or not isinstance(reply.get("material_after_reply_cp"), int)
                    or not isinstance(reply.get("piece_present"), bool)
                    or not USI_MOVE.fullmatch(reply["reply"])):
                raise ValueError("checking reply lacks a bound legal position")
            selected.append({"case_id": row["case_id"], "sfen": case["sfen"],
                             "candidate": row["candidate"], "reply": reply["reply"],
                             "reply_hash": board_hash,
                             "material_after_reply_cp": reply["material_after_reply_cp"],
                             "gold_present": reply["piece_present"]})
    if len(selected) != 7:
        raise ValueError("expected seven checking replies in guarded pair")
    return selected


def parse_legal_moves(stdout: str) -> list[str]:
    lines = stdout.splitlines()
    if len(lines) != 1:
        raise ValueError("legal-moves helper must emit one line")
    moves = lines[0].split()
    if len(set(moves)) != len(moves) or any(not USI_MOVE.fullmatch(move) for move in moves):
        raise ValueError("legal-moves helper returned invalid or duplicate moves")
    return moves


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cases", type=Path, required=True)
    parser.add_argument("--spec", type=Path, required=True)
    parser.add_argument("--ledger", type=Path, required=True)
    parser.add_argument("--history-bin", type=Path, required=True)
    parser.add_argument("--legal-bin", type=Path, required=True)
    parser.add_argument("--max-total-seconds", type=int, default=20)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if (not 1 <= args.max_total_seconds <= 30 or not args.history_bin.is_file()
            or not args.legal_bin.is_file()):
        parser.error("requires existing helpers and a 1..=30 second cap")
    try:
        cases = load_cases(args.cases)
        spec = json.loads(args.spec.read_text(encoding="utf-8"))
        if (cases.get("diagnostic_only") is not True
                or spec.get("selections") != [
                    {"case_id": "pinned-guarded-focus", "candidate": "5h4h", "origin": "5g"},
                    {"case_id": "pinned-guarded-contrast", "candidate": "5h4h", "origin": "5g"}]):
            raise ValueError("input is not the guarded focus and contrast pair")
        ledger = json.loads(args.ledger.read_text(encoding="utf-8"))
        selected = select_checks(ledger, cases, sha256(args.cases), sha256(args.spec))
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
            moves_result = subprocess.run([str(args.legal_bin)], input=reply_sfen + "\n",
                                          text=True, capture_output=True, check=False,
                                          timeout=remaining)
            if moves_result.returncode:
                raise RuntimeError(f"legal-moves helper exited {moves_result.returncode}: "
                                   + moves_result.stderr[:300])
            moves = parse_legal_moves(moves_result.stdout)
            rows.append({key: value for key, value in item.items() if key != "sfen"}
                        | {"legal_evasions": moves,
                           "king_evasions": [move for move in moves if move.startswith("4h")],
                           "checkmate_now": not moves})
        except (subprocess.TimeoutExpired, TimeoutError, RuntimeError, ValueError,
                KeyError, OSError) as error:
            failure = {"case_id": item["case_id"], "reply": item["reply"],
                       "reason": type(error).__name__, "detail": str(error)[:500]}
            break
    report = {"schema": "sekirei.guarded-check-evasions.v1",
              "diagnostic_only": True, "strength_claim": False,
              "scope": "legal_evasions_after_every_guarded_checking_reply",
              "status": "complete" if failure is None else "interrupted",
              "planned_rows": len(selected), "completed_rows": len(rows), "failure": failure,
              "inputs": {"cases": str(args.cases), "cases_sha256": sha256(args.cases),
                         "spec": str(args.spec), "spec_sha256": sha256(args.spec),
                         "ledger": str(args.ledger), "ledger_sha256": sha256(args.ledger),
                         "history_bin": str(args.history_bin),
                         "history_bin_sha256": sha256(args.history_bin),
                         "legal_bin": str(args.legal_bin),
                         "legal_bin_sha256": sha256(args.legal_bin),
                         "runner_sha256": sha256(Path(__file__).resolve()),
                         "max_total_seconds": args.max_total_seconds},
              "rows": rows}
    atomic_json(args.output, report)
    print(json.dumps({"status": report["status"], "rows": len(rows),
                      "output": str(args.output)}, sort_keys=True))
    return 0 if failure is None else 1


if __name__ == "__main__":
    raise SystemExit(main())
