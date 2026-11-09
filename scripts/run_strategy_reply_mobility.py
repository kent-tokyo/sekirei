#!/usr/bin/env python3
"""Measure one piece's legal options after each actual opponent reply.

One original case per process, at most four cases and 64 replies per case.
The optional material ledger enumerates every immediate legal recapture.
Neither mode establishes positional move value.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import time
from pathlib import Path

from run_strategy_piece_mobility import SCHEMA as SELECTION_SCHEMA
from run_tactical_line_diagnostic import atomic_json, load_cases, run_process, sha256


FIELDS = {"candidate", "origin", "initial_hash", "child_hash", "status", "before",
          "max_replies", "replies", "min", "max", "removed", "checking",
          "lost_without_recapture", "details", "board_restored"}
MATERIAL_FIELDS = {"material_before_cp", "material_after_candidate_cp", "material_details"}


def parse_probe(stdout: str, candidate: str, origin: str, cap: int,
                material_ledger: bool = False) -> dict:
    lines = stdout.splitlines()
    if len(lines) != 1:
        raise ValueError("reply mobility probe must emit exactly one line")
    parts = lines[0].split("\t")
    if any("=" not in part for part in parts):
        raise ValueError("malformed reply mobility field")
    fields = dict(part.split("=", 1) for part in parts)
    expected_fields = FIELDS | MATERIAL_FIELDS if material_ledger else FIELDS
    if (len(parts) != len(expected_fields) or set(fields) != expected_fields
            or fields["candidate"] != candidate or fields["origin"] != origin
            or fields["max_replies"] != str(cap) or fields["board_restored"] != "true"):
        raise ValueError("reply mobility probe returned mismatched fields or unrestored board")
    if any(len(fields[key]) != 16 or any(c not in "0123456789abcdef" for c in fields[key])
           for key in ("initial_hash", "child_hash")):
        raise ValueError("invalid board hash")
    before, replies, removed, checking, lost_without_recapture = (
        int(fields[key]) for key in ("before", "replies", "removed", "checking", "lost_without_recapture"))
    if (min(before, replies, removed, checking, lost_without_recapture) < 0
            or removed > replies or checking > replies or lost_without_recapture > removed):
        raise ValueError("invalid mobility count")
    status = fields["status"]
    if status not in ("complete", "no_legal_reply", "unknown_reply_cap"):
        raise ValueError("invalid mobility status")
    rows = []
    if fields["details"]:
        for item in fields["details"].split(","):
            move, count, state, check_text, recaptures = item.split(":")
            count = int(count)
            recaptures = int(recaptures)
            if (not move or count < 0 or state not in ("present", "removed")
                    or check_text not in ("true", "false") or recaptures < 0
                    or (state == "removed" and count) or (state == "present" and recaptures)):
                raise ValueError("invalid reply mobility detail")
            rows.append({"reply": move, "count": count, "piece_present": state == "present",
                         "in_check": check_text == "true", "legal_recapture_count": recaptures})
    if len({row["reply"] for row in rows}) != len(rows):
        raise ValueError("duplicate reply")
    if status == "unknown_reply_cap":
        if (replies <= cap or rows or fields["min"] != "none" or fields["max"] != "none"
                or removed or checking or lost_without_recapture):
            raise ValueError("capped result must remain unknown")
        minimum = maximum = None
    elif status == "no_legal_reply":
        if (replies or rows or fields["min"] != "none" or fields["max"] != "none"
                or removed or checking or lost_without_recapture):
            raise ValueError("terminal result must have no reply counts")
        minimum = maximum = None
    else:
        if not (0 < replies <= cap and len(rows) == replies):
            raise ValueError("complete result requires every reply")
        minimum, maximum = min(row["count"] for row in rows), max(row["count"] for row in rows)
        if (fields["min"] != str(minimum) or fields["max"] != str(maximum)
                or removed != sum(not row["piece_present"] for row in rows)
                or checking != sum(row["in_check"] for row in rows)
                or lost_without_recapture != sum(not row["piece_present"] and not row["legal_recapture_count"]
                                                 for row in rows)):
            raise ValueError("reply aggregates do not match details")
    result = {"candidate": candidate, "origin": origin, "status": status,
            "initial_hash": fields["initial_hash"], "child_hash": fields["child_hash"],
            "before_count": before, "reply_count": replies, "min_count": minimum,
            "max_count": maximum, "removed_count": removed, "checking_reply_count": checking,
            "lost_without_recapture_count": lost_without_recapture, "reply_rows": rows}
    if material_ledger:
        material_before = int(fields["material_before_cp"])
        material_after_candidate = int(fields["material_after_candidate_cp"])
        detail_items = fields["material_details"].split(",") if fields["material_details"] else []
        if len(detail_items) != len(rows):
            raise ValueError("material detail count differs from replies")
        for row, item in zip(rows, detail_items):
            reply, score_text, reply_hash, recapture_text = item.split(":")
            if reply != row["reply"]:
                raise ValueError("material detail reply order differs")
            if len(reply_hash) != 16 or any(c not in "0123456789abcdef" for c in reply_hash):
                raise ValueError("invalid material reply hash")
            row["material_after_reply_cp"] = int(score_text)
            row["reply_hash"] = reply_hash
            recaptures = []
            if recapture_text:
                for entry in recapture_text.split("|"):
                    move, score = entry.split("@")
                    recaptures.append({"move": move, "material_after_recapture_cp": int(score)})
            if (len(recaptures) != row["legal_recapture_count"]
                    or len({entry["move"] for entry in recaptures}) != len(recaptures)
                    or any(not entry["move"] for entry in recaptures)):
                raise ValueError("material recapture count differs from legal replies")
            row["recapture_material"] = recaptures
        result.update({"material_before_cp": material_before,
                       "material_after_candidate_cp": material_after_candidate})
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cases", required=True, type=Path)
    parser.add_argument("--spec", required=True, type=Path)
    parser.add_argument("--probe-bin", required=True, type=Path)
    parser.add_argument("--max-replies", type=int, default=64)
    parser.add_argument("--max-total-seconds", type=int, default=30)
    parser.add_argument("--material-ledger", action="store_true")
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if not args.probe_bin.is_file():
        parser.error("probe binary does not exist")
    if not 1 <= args.max_replies <= 64 or not 1 <= args.max_total_seconds <= 30:
        parser.error("caps must be 1..=64 replies and 1..=30 seconds")
    cases = load_cases(args.cases)
    by_id = {case["id"]: case for case in cases["cases"]}
    spec = json.loads(args.spec.read_text(encoding="utf-8"))
    selections = spec.get("selections")
    if spec.get("schema") != SELECTION_SCHEMA or not isinstance(selections, list) or not 1 <= len(selections) <= 4:
        parser.error("invalid or oversized mobility selection spec")
    tasks = []
    seen = set()
    for selection in selections:
        if not isinstance(selection, dict) or set(selection) != {"case_id", "candidate", "origin"}:
            parser.error("invalid mobility selection")
        case_id, candidate, origin = (selection[key] for key in ("case_id", "candidate", "origin"))
        if (not isinstance(case_id, str) or case_id not in by_id or case_id in seen
                or not isinstance(candidate, str) or not isinstance(origin, str)):
            parser.error("unknown or duplicate mobility selection")
        seen.add(case_id)
        case = by_id[case_id]
        if (len(case["stages"]) != 1 or case["stages"][0].get("moves")
                or case["stages"][0].get("expected_sfen", case["sfen"]) != case["sfen"]
                or candidate not in case["stages"][0].get("candidates", [])):
            parser.error("selection must refer to a listed direct-position candidate")
        tasks.append((case_id, case["sfen"], candidate, origin))
    deadline = time.monotonic() + args.max_total_seconds
    rows = []
    failure = None
    for case_id, sfen, candidate, origin in tasks:
        try:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("reply mobility batch deadline exhausted")
            command = [str(args.probe_bin), "--sfen", sfen, "--candidate", candidate,
                       "--origin", origin, "--max-replies", str(args.max_replies)]
            if args.material_ledger:
                command.append("--material-ledger")
            row = parse_probe(run_process(command, timeout=remaining).stdout, candidate, origin,
                              args.max_replies, args.material_ledger)
            rows.append({"case_id": case_id, **row})
        except (subprocess.TimeoutExpired, TimeoutError, RuntimeError, ValueError) as error:
            failure = {"case_id": case_id, "candidate": candidate, "origin": origin,
                       "reason": type(error).__name__, "detail": str(error)[:500]}
            break
    status = "complete" if failure is None else "interrupted"
    report = {"schema": "sekirei.strategy-reply-mobility-report.v4" if args.material_ledger
              else "sekirei.strategy-reply-mobility-report.v2",
              "diagnostic_only": True, "strength_claim": False,
              "scope": "material_after_reply_and_immediate_recaptures" if args.material_ledger
              else "selected_piece_after_each_actual_legal_opponent_reply",
              "status": status, "planned_rows": len(tasks), "completed_rows": len(rows),
              "failure": failure,
              "inputs": {"cases": str(args.cases), "cases_sha256": sha256(args.cases),
                         "spec": str(args.spec), "spec_sha256": sha256(args.spec),
                         "probe_bin": str(args.probe_bin), "probe_bin_sha256": sha256(args.probe_bin),
                         **({"material_ledger": True} if args.material_ledger else {}),
                         "max_replies": args.max_replies,
                         "max_total_seconds": args.max_total_seconds}, "rows": rows}
    atomic_json(args.output, report)
    print(json.dumps({"rows": len(rows), "status": status, "output": str(args.output)}, sort_keys=True))
    return 0 if failure is None else 1


if __name__ == "__main__":
    raise SystemExit(main())
