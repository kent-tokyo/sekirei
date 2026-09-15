#!/usr/bin/env python3
"""Re-search a Floodgate diagnostic corpus at a fixed node budget.

This is a diagnostic runner, not a strength gate.  Every position is run in a
fresh USI process so TT/history state cannot leak between positions.  The
observed game move is reported for review and is never called a correct move.
"""
from __future__ import annotations

import argparse
import json
import queue
import re
import subprocess
import threading
import time
from pathlib import Path


INFO_NODES = re.compile(r"\binfo .*?depth (\d+) .*?score (?:cp )?(-?\d+) .*?nodes (\d+)")


def parse_output(output: str) -> dict:
    bestmove = None
    scores = []
    for line in output.splitlines():
        if line.startswith("bestmove "):
            bestmove = line.split()[1]
        match = INFO_NODES.search(line)
        if match:
            scores.append({
                "depth": int(match.group(1)),
                "score_cp": int(match.group(2)),
                "nodes": int(match.group(3)),
            })
    return {"bestmove_usi": bestmove, "last_info": scores[-1] if scores else None}


def run_position(binary: Path, sfen: str, search_command: str, timeout: float, weights: Path | None) -> dict:
    commands = [
        "usi",
        "setoption name Threads value 1",
        "setoption name SpecTopN value 0",
        "setoption name SearchMode value Speculative",
        "setoption name UseBook value false",
    ]
    if weights is not None:
        commands.append(f"setoption name EvalFile value {weights}")
    commands.extend(["isready", f"position sfen {sfen}", f"go {search_command}"])
    process = subprocess.Popen(
        [str(binary)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
        stderr=subprocess.PIPE, text=True, bufsize=1,
    )
    assert process.stdin is not None and process.stdout is not None
    # Do not combine selectors with TextIOWrapper.readline: it can leave later
    # USI lines in Python's user-space buffer while the OS fd looks idle. A
    # dedicated blocking reader preserves every line, and `quit` is sent only
    # after the search has emitted bestmove (not immediately after `go`).
    lines: queue.Queue[str | None] = queue.Queue()

    def drain_stdout() -> None:
        for line in process.stdout:
            lines.put(line)
        lines.put(None)

    reader = threading.Thread(target=drain_stdout, daemon=True)
    reader.start()
    process.stdin.write("\n".join(commands) + "\n")
    process.stdin.flush()
    output: list[str] = []
    timed_out = False
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            line = lines.get(timeout=max(0.0, deadline - time.monotonic()))
        except queue.Empty:
            break
        if line is None:
            break
        output.append(line)
        if line.startswith("bestmove "):
            process.stdin.write("quit\n")
            process.stdin.flush()
            break
    else:
        timed_out = True
    if not output or not any(line.startswith("bestmove ") for line in output):
        timed_out = True
    if timed_out:
        process.kill()
    try:
        process.wait(timeout=2)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait()
    reader.join(timeout=1)
    stderr = process.stderr.read() if process.stderr is not None else ""
    parsed = parse_output("".join(output))
    parsed.update({"returncode": process.returncode, "stderr": stderr[-1000:]})
    # A missing bestmove is always an incomplete/timeout observation for this
    # runner, regardless of whether the child exited by signal or was killed
    # after the watchdog deadline.
    parsed["timed_out"] = timed_out or parsed["bestmove_usi"] is None
    return parsed


def run_corpus(binary: Path, corpus: dict, search_command: str, timeout: float,
               weights: Path | None, limit: int | None) -> dict:
    entries = corpus.get("entries", [])
    if limit is not None:
        entries = entries[:limit]
    results = []
    for index, entry in enumerate(entries):
        position = entry["position"]
        result = run_position(binary, position["sfen"], search_command, timeout, weights)
        results.append({
            "index": index,
            "source": entry["source"],
            "observed_move_csa": position.get("actual_move_observed"),
            "engine_move_usi": result["bestmove_usi"],
            "last_info": result["last_info"],
            "completion": (
                "timeout" if result["timed_out"] else
                "depth_completed" if result["last_info"] else
                "bestmove_without_depth_info" if search_command.startswith("depth ") else
                "budget_before_depth_completion"
            ),
            "returncode": result["returncode"],
            "stderr_tail": result["stderr"],
            "observed_move_is_label": False,
        })
    return {
        "schema": "sekirei.floodgate-fixed-node-rerun.v1",
        "diagnostic_only": True,
        "search_command": search_command,
        "threads": 1,
        "spec_top_n": 0,
        "use_book": False,
        "weights": str(weights) if weights else "not_supplied",
        "results": results,
        "strength_claim": "not_permitted",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--corpus", type=Path, required=True)
    budget = parser.add_mutually_exclusive_group()
    budget.add_argument("--nodes", type=int)
    budget.add_argument("--depth", type=int)
    parser.add_argument("--timeout-seconds", type=float, default=10.0)
    parser.add_argument("--weights", type=Path)
    parser.add_argument("--limit", type=int)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.timeout_seconds <= 0 or (args.nodes is not None and args.nodes <= 0) or (args.depth is not None and args.depth <= 0):
        parser.error("nodes/depth and timeout must be positive")
    if args.nodes is None and args.depth is None:
        args.nodes = 20_000
    search_command = f"nodes {args.nodes}" if args.nodes is not None else f"depth {args.depth}"
    corpus = json.loads(args.corpus.read_text(encoding="utf-8"))
    result = run_corpus(args.binary, corpus, search_command, args.timeout_seconds, args.weights, args.limit)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {len(result['results'])} diagnostic reruns")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
