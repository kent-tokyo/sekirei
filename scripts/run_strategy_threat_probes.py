#!/usr/bin/env python3
"""Bound one-ply threat probes over original direct-position candidates.

Every candidate gets a fresh process. A complete result proves only whether
all legal replies permit an immediate mate; it does not rank candidate moves.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import time
from pathlib import Path

from run_tactical_line_diagnostic import atomic_json, load_cases, run_process, sha256


STATUSES = {
    "all_replies_allow_mate_in_one", "has_escape_from_mate_in_one",
    "mate_now", "no_legal_reply", "unknown_reply_cap", "unknown_invalid_reply",
}
FIELDS = {
    "move", "initial_hash", "child_hash", "status", "max_replies", "replies",
    "replies_with_mate_in_one", "first_escape", "gives_check", "is_capture",
    "board_restored",
}


def parse_probe(stdout: str, candidate: str, max_replies: int) -> dict:
    lines = stdout.splitlines()
    if len(lines) != 1:
        raise ValueError("threat probe must emit exactly one line")
    fields = dict(field.split("=", 1) for field in lines[0].split("\t"))
    if set(fields) != FIELDS or fields["move"] != candidate:
        raise ValueError("threat probe returned mismatched fields or move")
    if fields["max_replies"] != str(max_replies) or fields["board_restored"] != "true":
        raise ValueError("threat probe returned mismatched cap or unrestored board")
    if fields["status"] not in STATUSES:
        raise ValueError("threat probe returned an invalid status")
    if fields["gives_check"] not in ("true", "false") or fields["is_capture"] not in ("true", "false"):
        raise ValueError("threat probe returned an invalid boolean")
    if any(len(fields[key]) != 16 or not all(c in "0123456789abcdef" for c in fields[key])
           for key in ("initial_hash", "child_hash")):
        raise ValueError("threat probe returned an invalid hash")
    replies = int(fields["replies"])
    mate_replies = int(fields["replies_with_mate_in_one"])
    if replies < 0 or mate_replies < 0 or mate_replies > replies:
        raise ValueError("threat probe returned invalid reply counts")
    status = fields["status"]
    if status == "all_replies_allow_mate_in_one" and not (
        0 < replies == mate_replies and fields["first_escape"] == "none"
    ):
        raise ValueError("complete threat requires every legal reply")
    if status == "has_escape_from_mate_in_one" and not (
        0 < replies and mate_replies < replies and fields["first_escape"] != "none"
    ):
        raise ValueError("escape result requires a counterexample")
    if status == "unknown_reply_cap" and replies <= max_replies:
        raise ValueError("reply cap status requires a larger reply set")
    if status in ("mate_now", "no_legal_reply") and replies != 0:
        raise ValueError("terminal status requires zero replies")
    if status == "mate_now" and fields["gives_check"] != "true":
        raise ValueError("mate-now status requires check")
    return {
        "candidate": candidate, "initial_hash": fields["initial_hash"],
        "child_hash": fields["child_hash"], "status": status,
        "replies": replies, "replies_with_mate_in_one": mate_replies,
        "first_escape": fields["first_escape"],
        "gives_check": fields["gives_check"] == "true",
        "is_capture": fields["is_capture"] == "true",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cases", type=Path, required=True)
    parser.add_argument("--probe-bin", type=Path, required=True)
    parser.add_argument("--max-replies", type=int, default=64)
    parser.add_argument("--max-invocations", type=int, required=True)
    parser.add_argument("--max-total-seconds", type=int, default=90)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if not args.probe_bin.is_file():
        parser.error(f"probe binary does not exist: {args.probe_bin}")
    if not 1 <= args.max_replies <= 128:
        parser.error("--max-replies must be 1..=128")
    if not 1 <= args.max_invocations <= 24 or not 1 <= args.max_total_seconds <= 90:
        parser.error("--max-invocations must be 1..=24 and --max-total-seconds 1..=90")
    cases = load_cases(args.cases)
    tasks = []
    for case in cases["cases"]:
        for stage in case["stages"]:
            if stage.get("moves") or stage.get("expected_sfen", case["sfen"]) != case["sfen"]:
                parser.error("this runner accepts only direct, unreplayed case positions")
            tasks.extend((case["id"], stage["id"], case["sfen"], candidate)
                         for candidate in stage.get("candidates", []))
    if not tasks or len(tasks) > args.max_invocations:
        parser.error(f"batch needs {len(tasks)} invocations; cap is {args.max_invocations}")
    deadline = time.monotonic() + args.max_total_seconds
    rows = []
    failure = None
    for case_id, stage_id, sfen, candidate in tasks:
        try:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("threat-probe batch deadline exhausted")
            command = [str(args.probe_bin), "--sfen", sfen, "--move", candidate,
                       "--max-replies", str(args.max_replies)]
            row = parse_probe(run_process(command, timeout=remaining).stdout,
                              candidate, args.max_replies)
            rows.append({"case_id": case_id, "stage_id": stage_id, **row})
        except (subprocess.TimeoutExpired, TimeoutError, RuntimeError, ValueError) as error:
            failure = {"case_id": case_id, "stage_id": stage_id, "candidate": candidate,
                       "reason": type(error).__name__, "detail": str(error)[:500]}
            break
    status = "complete" if failure is None else "interrupted"
    report = {
        "schema": "sekirei.strategy-one-ply-threat-probes.v1",
        "diagnostic_only": True, "strength_claim": False,
        "scope": "after_candidate_all_legal_replies_allow_mate_in_one",
        "status": status, "completed_invocations": len(rows),
        "planned_invocations": len(tasks), "failure": failure,
        "inputs": {"cases": str(args.cases), "cases_sha256": sha256(args.cases),
                   "probe_bin": str(args.probe_bin),
                   "probe_bin_sha256": sha256(args.probe_bin),
                   "max_replies": args.max_replies,
                   "max_invocations": args.max_invocations,
                   "max_total_seconds": args.max_total_seconds},
        "rows": rows,
    }
    atomic_json(args.output, report)
    print(json.dumps({"rows": len(rows), "status": status,
                      "output": str(args.output)}, sort_keys=True))
    return 0 if status == "complete" else 1


if __name__ == "__main__":
    raise SystemExit(main())
