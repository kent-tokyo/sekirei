#!/usr/bin/env python3
"""Run a small fixed-depth USI comparison between Speculative and LazySMP."""

from __future__ import annotations

import argparse
import json
import subprocess
from pathlib import Path


POSITIONS = {
    "startpos": "startpos",
    "opening_two_ply": "sfen lnsgkgsnl/1r5b1/ppppppppp/9/2P6/9/PP1PPPPPP/1B5R1/LNSGKGSNL b - 3",
    "capture_and_drop": "sfen 4k4/9/9/9/4R4/9/9/9/4K4 b P 1",
}


def parse_info_line(line: str) -> dict[str, object] | None:
    """Parse the stable key/value part of a USI ``info`` line.

    Do not rely on absolute token offsets: optional score bounds, multipv, or
    additional engine fields may appear before ``nodes`` and ``time``.
    """
    fields = line.split()
    if len(fields) < 3 or fields[:2] != ["info", "depth"]:
        return None

    def value_after(key: str) -> str:
        try:
            index = fields.index(key)
            return fields[index + 1]
        except (ValueError, IndexError) as exc:
            raise ValueError(f"missing value for {key!r}: {line!r}") from exc

    try:
        score_index = fields.index("score")
        score_kind = fields[score_index + 1]
        score_value = fields[score_index + 2]
        if score_kind not in {"cp", "mate"}:
            raise ValueError(f"unsupported score kind {score_kind!r}: {line!r}")
        return {
            "depth": int(value_after("depth")),
            "score": f"{score_kind} {score_value}",
            "nodes": int(value_after("nodes")),
            "time_ms": int(value_after("time")),
        }
    except (ValueError, IndexError) as exc:
        if isinstance(exc, ValueError) and str(exc).startswith(("missing value", "unsupported score")):
            raise
        raise ValueError(f"malformed score in info line: {line!r}") from exc


def run(binary: Path, mode: str, position: str, search_command: str, threads: int) -> dict[str, object]:
    process = subprocess.Popen(
        [str(binary)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL, text=True, bufsize=1,
    )
    assert process.stdin is not None and process.stdout is not None
    commands = [
        "usi", "isready", "setoption name UseBook value false",
        f"setoption name Threads value {threads}",
        f"setoption name SearchMode value {mode}",
        f"position {position}", f"go {search_command}",
    ]
    for command in commands:
        process.stdin.write(command + "\n")
        process.stdin.flush()
    info = None
    bestmove = None
    for line in process.stdout:
        if line.startswith("info depth"):
            info = parse_info_line(line.strip())
        if line.startswith("bestmove"):
            bestmove = line.strip()
            break
    process.stdin.write("quit\n")
    process.stdin.flush()
    process.wait(timeout=10)
    if info is None or bestmove is None or process.returncode != 0:
        raise RuntimeError(f"incomplete run: mode={mode} info={info!r} bestmove={bestmove!r}")
    return {
        "mode": mode, "position": position, "depth": info["depth"],
        "score": info["score"], "nodes": info["nodes"],
        "time_ms": info["time_ms"], "bestmove": bestmove.split()[1],
        "budget_scope": "per_worker",
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, default=Path("target/release/sekirei"))
    budget = parser.add_mutually_exclusive_group()
    budget.add_argument("--depth", type=int, default=4)
    budget.add_argument("--nodes", type=int)
    parser.add_argument("--repeats", type=int, default=1)
    parser.add_argument("--output", type=Path, help="write the JSON report to this path")
    args = parser.parse_args()
    if args.nodes is not None and args.nodes <= 0:
        parser.error("--nodes must be positive")
    if args.repeats <= 0:
        parser.error("--repeats must be positive")
    search_command = f"nodes {args.nodes}" if args.nodes is not None else f"depth {args.depth}"
    records = []
    for repeat in range(1, args.repeats + 1):
        for name, position in POSITIONS.items():
            for mode, threads in (("Speculative", 1), ("LazySMP", 2)):
                record = run(args.binary, mode, position, search_command, threads)
                record["case"] = name
                record["repeat"] = repeat
                record["budget"] = search_command
                records.append(record)
    report = {"schema": "sekirei.lazy-smp-fixed-depth.v1", "records": records}
    text = json.dumps(report, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(text, encoding="utf-8")
    print(text, end="")


if __name__ == "__main__":
    main()
