#!/usr/bin/env python3
"""Record bounded two-worker USI selected-move/PV coherence for one case."""

from __future__ import annotations

import argparse
import asyncio
import json
import time
from pathlib import Path

from nps_threads import parse_lazy_smp
from run_tactical_line_diagnostic import atomic_json, load_cases, sha256


async def read_until(proc: asyncio.subprocess.Process, prefix: str, deadline: float) -> list[str]:
    lines = []
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("USI search deadline exhausted")
        raw = await asyncio.wait_for(proc.stdout.readline(), timeout=remaining)
        if not raw:
            raise RuntimeError("USI process ended before " + prefix)
        line = raw.decode("utf-8", errors="replace").strip()
        lines.append(line)
        if line.startswith(prefix):
            return lines


def parse_search(lines: list[str]) -> dict:
    diagnostic = parse_lazy_smp(lines)
    if (diagnostic is None or diagnostic.get("workers") != 2
            or len(diagnostic.get("worker_stats", [])) != 2):
        raise ValueError("missing two-worker Lazy SMP diagnostic")
    if diagnostic.get("selected_move_matches_pv_root") is None:
        raise ValueError("missing completed-iteration coherence field")
    if (len(diagnostic.get("worker_bestmoves", [])) != 2
            or len(diagnostic.get("worker_pv_roots", [])) != 2
            or len(diagnostic.get("worker_move_matches_pv_root", [])) != 2):
        raise ValueError("missing per-worker move/PV coherence fields")
    scored_lines = [line.split() for line in lines if line.startswith("info depth ")]
    if len(scored_lines) != 1 or "pv" not in scored_lines[0]:
        raise ValueError("expected one scored USI PV line")
    scored = scored_lines[0]
    try:
        scored_depth = int(scored[2])
        scored_pv_root = scored[scored.index("pv") + 1]
    except (ValueError, IndexError) as error:
        raise ValueError("malformed scored USI PV line") from error
    if (scored_depth != diagnostic["worker_stats"][diagnostic["selected_worker"]]["depth"]
            or scored_pv_root != diagnostic["selected_pv_root"]):
        raise ValueError("scored USI PV differs from selected completed iteration")
    bestmoves = [line.split() for line in lines if line.startswith("bestmove ")]
    if len(bestmoves) != 1 or len(bestmoves[0]) < 2:
        raise ValueError("expected one USI bestmove")
    if bestmoves[0][1] != diagnostic["selected_bestmove"]:
        raise ValueError("selected worker move differs from final bestmove")
    return {"selected_worker": diagnostic["selected_worker"],
            "selected_bestmove": diagnostic["selected_bestmove"],
            "selected_pv_root": diagnostic["selected_pv_root"],
            "usi_scored_pv_root": scored_pv_root,
            "matches_completed_pv": diagnostic["selected_move_matches_pv_root"],
            "worker_bestmoves": diagnostic["worker_bestmoves"],
            "worker_pv_roots": diagnostic["worker_pv_roots"],
            "worker_move_matches_pv_root": diagnostic["worker_move_matches_pv_root"],
            "move_agreement": diagnostic["move_agreement"],
            "worker_depths": [worker["depth"] for worker in diagnostic["worker_stats"]],
            "worker_aborted": [worker["aborted"] for worker in diagnostic["worker_stats"]]}


async def search_once(binary: Path, sfen: str, nodes: int, deadline: float) -> dict:
    proc = await asyncio.create_subprocess_exec(str(binary.resolve()),
                                                stdin=asyncio.subprocess.PIPE,
                                                stdout=asyncio.subprocess.PIPE,
                                                stderr=asyncio.subprocess.PIPE)
    try:
        proc.stdin.write(b"usi\n")
        await proc.stdin.drain()
        await read_until(proc, "usiok", deadline)
        proc.stdin.write(b"setoption name Threads value 2\n"
                         b"setoption name SearchMode value LazySMP\n"
                         b"isready\n")
        await proc.stdin.drain()
        await read_until(proc, "readyok", deadline)
        proc.stdin.write(f"position sfen {sfen}\ngo nodes {nodes}\n".encode())
        await proc.stdin.drain()
        lines = await read_until(proc, "bestmove ", deadline)
        return parse_search(lines)
    finally:
        if proc.returncode is None:
            try:
                proc.stdin.write(b"quit\n")
                await proc.stdin.drain()
                await asyncio.wait_for(proc.wait(), timeout=1)
            except (BrokenPipeError, ConnectionResetError, asyncio.TimeoutError):
                proc.kill()
                await proc.wait()


async def run(args: argparse.Namespace) -> int:
    cases = load_cases(args.cases)
    selected = [case for case in cases["cases"] if case["id"] == args.case_id]
    if len(selected) != 1 or len(selected[0]["stages"]) != 1 or selected[0]["stages"][0].get("moves"):
        raise ValueError("case must identify one direct-position stage")
    sfen = selected[0]["sfen"]
    deadline = time.monotonic() + args.max_total_seconds
    rows = []
    failure = None
    for index in range(args.repeats):
        try:
            rows.append({"repetition": index + 1,
                         **await search_once(args.engine, sfen, args.nodes, deadline)})
        except (TimeoutError, asyncio.TimeoutError, RuntimeError, ValueError, OSError) as error:
            failure = {"repetition": index + 1, "reason": type(error).__name__,
                       "detail": str(error)[:500]}
            break
    status = "complete" if failure is None else "interrupted"
    report = {"schema": "sekirei.lazy-selected-coherence.v3",
              "diagnostic_only": True, "strength_claim": False, "status": status,
              "completed_rows": len(rows), "planned_rows": args.repeats, "failure": failure,
              "inputs": {"cases": str(args.cases), "cases_sha256": sha256(args.cases),
                         "case_id": args.case_id, "sfen": sfen,
                         "engine": str(args.engine), "engine_sha256": sha256(args.engine),
                         "threads": 2, "nodes_per_worker": args.nodes,
                         "repeats": args.repeats,
                         "max_total_seconds": args.max_total_seconds}, "rows": rows}
    atomic_json(args.output, report)
    print(json.dumps({"status": status, "rows": len(rows), "output": str(args.output)}, sort_keys=True))
    return 0 if failure is None else 1


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cases", required=True, type=Path)
    parser.add_argument("--case-id", required=True)
    parser.add_argument("--engine", required=True, type=Path)
    parser.add_argument("--nodes", required=True, type=int)
    parser.add_argument("--repeats", required=True, type=int)
    parser.add_argument("--max-total-seconds", required=True, type=int)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if not args.engine.is_file():
        parser.error("engine does not exist")
    if not 1 <= args.nodes <= 20_000 or not 1 <= args.repeats <= 3 or not 1 <= args.max_total_seconds <= 15:
        parser.error("caps are 1..=20000 nodes, 1..=3 repeats and 1..=15 seconds")
    try:
        return asyncio.run(run(args))
    except ValueError as error:
        parser.error(str(error))


if __name__ == "__main__":
    raise SystemExit(main())
