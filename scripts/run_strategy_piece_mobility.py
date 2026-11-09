#!/usr/bin/env python3
"""Measure one piece's legal options before and after a legal setup move.

After the move, the original mover is hypothetically put back on turn. This
measures static move availability, not an actual consecutive turn or strength.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import time
from pathlib import Path

from generate_search_diagnostic import decode, square
from run_tactical_line_diagnostic import atomic_json, load_cases, run_process, sha256


SCHEMA = "sekirei.strategy-piece-mobility-spec.v1"


def same_side_sfen(child: str, side: str) -> str:
    fields = child.split()
    if len(fields) != 4 or fields[1] == side:
        raise ValueError("history helper did not switch the side to move")
    fields[1] = side
    return " ".join(fields)


def piece_at(sfen: str, origin: str) -> str | None:
    return decode(sfen)[0][square(origin)]


def legal_moves_for_two(binary: Path, initial: str, after_same_side: str,
                        timeout: float) -> tuple[list[str], list[str]]:
    command = [str(binary)]
    result = subprocess.run(command, input=f"{initial}\n{after_same_side}\n",
                            text=True, capture_output=True, check=False, timeout=timeout)
    if result.returncode:
        raise RuntimeError(f"legal-move helper failed ({result.returncode}): {result.stderr[-500:]}")
    lines = result.stdout.splitlines()
    if len(lines) != 2:
        raise ValueError("legal-move helper must return two position lines")
    return ([move for move in lines[0].split()], [move for move in lines[1].split()])


def measure(case: dict, candidate: str, origin: str, legal_bin: Path,
            history_bin: Path, deadline: float) -> dict:
    initial = case["sfen"]
    side = initial.split()[1]
    if len(origin) != 2 or origin[0] not in "123456789" or origin[1] not in "abcdefghi":
        raise ValueError("origin must be a USI square")
    piece = piece_at(initial, origin)
    if piece is None or (piece[-1].isupper() != (side == "b")):
        raise ValueError("origin must hold a piece of the mover")
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise TimeoutError("mobility batch deadline exhausted")
    command = [str(history_bin), "--sfen", initial, "--moves", candidate]
    fields = dict(item.split("=", 1) for item in run_process(command, timeout=remaining).stdout.strip().split("\t"))
    if set(fields) != {"sfen", "hash"}:
        raise ValueError("history helper returned an unexpected contract")
    child = fields["sfen"]
    if piece_at(child, origin) != piece:
        raise ValueError("selected origin piece moved or changed; this probe requires it to stay")
    after = same_side_sfen(child, side)
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise TimeoutError("mobility batch deadline exhausted")
    before_moves, after_moves = legal_moves_for_two(legal_bin, initial, after, remaining)
    if candidate not in before_moves:
        raise ValueError("candidate was not legal in the original position")
    before = sorted(move for move in before_moves if move.startswith(origin))
    after_options = sorted(move for move in after_moves if move.startswith(origin))
    return {
        "case_id": case["id"], "candidate": candidate, "origin": origin,
        "piece": piece, "initial_sfen": initial, "child_sfen": child,
        "child_hash": fields["hash"], "hypothetical_same_side_sfen": after,
        "before_legal_moves": before, "after_legal_moves": after_options,
        "before_count": len(before), "after_count": len(after_options),
        "delta": len(after_options) - len(before),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cases", required=True, type=Path)
    parser.add_argument("--spec", required=True, type=Path)
    parser.add_argument("--legal-moves", required=True, type=Path)
    parser.add_argument("--history-sfen", required=True, type=Path)
    parser.add_argument("--max-invocations", required=True, type=int)
    parser.add_argument("--max-total-seconds", default=30, type=int)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    for executable in (args.legal_moves, args.history_sfen):
        if not executable.is_file():
            parser.error(f"executable does not exist: {executable}")
    if not 1 <= args.max_total_seconds <= 30 or not 1 <= args.max_invocations <= 16:
        parser.error("--max-total-seconds must be 1..=30 and --max-invocations 1..=16")
    cases = load_cases(args.cases)
    by_id = {case["id"]: case for case in cases["cases"]}
    spec = json.loads(args.spec.read_text(encoding="utf-8"))
    selections = spec.get("selections")
    if spec.get("schema") != SCHEMA or not isinstance(selections, list) or not selections:
        parser.error("invalid mobility spec")
    tasks = []
    seen = set()
    for selection in selections:
        if not isinstance(selection, dict):
            parser.error("each selection must be an object")
        case_id = selection.get("case_id")
        candidate = selection.get("candidate")
        origin = selection.get("origin")
        if (not isinstance(case_id, str) or case_id in seen or case_id not in by_id
                or not isinstance(origin, str)):
            parser.error("selection must have a unique known case and origin")
        seen.add(case_id)
        case = by_id[case_id]
        if (len(case["stages"]) != 1 or case["stages"][0].get("moves")
                or case["stages"][0].get("expected_sfen", case["sfen"]) != case["sfen"]):
            parser.error("mobility probe needs one direct stage")
        if candidate not in case["stages"][0].get("candidates", []):
            parser.error("selected candidate must be listed in the case")
        tasks.append((case, candidate, origin))
    if len(tasks) * 2 > args.max_invocations:
        parser.error(f"batch needs {len(tasks) * 2} subprocesses")
    deadline = time.monotonic() + args.max_total_seconds
    rows = []
    failure = None
    for case, candidate, origin in tasks:
        try:
            rows.append(measure(case, candidate, origin, args.legal_moves,
                                args.history_sfen, deadline))
        except (subprocess.TimeoutExpired, TimeoutError, RuntimeError, ValueError) as error:
            failure = {"case_id": case["id"], "candidate": candidate, "origin": origin,
                       "reason": type(error).__name__, "detail": str(error)[:500]}
            break
    status = "complete" if failure is None else "interrupted"
    report = {
        "schema": "sekirei.strategy-piece-mobility-report.v1",
        "diagnostic_only": True, "strength_claim": False,
        "same_side_after_is_hypothetical": True,
        "status": status, "planned_rows": len(tasks), "completed_rows": len(rows),
        "failure": failure,
        "inputs": {"cases": str(args.cases), "cases_sha256": sha256(args.cases),
                   "spec": str(args.spec), "spec_sha256": sha256(args.spec),
                   "legal_moves": str(args.legal_moves),
                   "legal_moves_sha256": sha256(args.legal_moves),
                   "history_sfen": str(args.history_sfen),
                   "history_sfen_sha256": sha256(args.history_sfen),
                   "max_invocations": args.max_invocations,
                   "max_total_seconds": args.max_total_seconds},
        "rows": rows,
    }
    atomic_json(args.output, report)
    print(json.dumps({"rows": len(rows), "status": status,
                      "output": str(args.output)}, sort_keys=True))
    return 0 if failure is None else 1


if __name__ == "__main__":
    raise SystemExit(main())
