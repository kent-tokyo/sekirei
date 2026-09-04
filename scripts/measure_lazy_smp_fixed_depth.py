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


def run(binary: Path, mode: str, position: str, depth: int, threads: int) -> dict[str, object]:
    process = subprocess.Popen(
        [str(binary)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL, text=True, bufsize=1,
    )
    assert process.stdin is not None and process.stdout is not None
    commands = [
        "usi", "isready", "setoption name UseBook value false",
        f"setoption name Threads value {threads}",
        f"setoption name SearchMode value {mode}",
        f"position {position}", f"go depth {depth}",
    ]
    for command in commands:
        process.stdin.write(command + "\n")
        process.stdin.flush()
    info = None
    bestmove = None
    for line in process.stdout:
        if line.startswith("info depth"):
            info = line.strip()
        if line.startswith("bestmove"):
            bestmove = line.strip()
            break
    process.stdin.write("quit\n")
    process.stdin.flush()
    process.wait(timeout=10)
    if info is None or bestmove is None or process.returncode != 0:
        raise RuntimeError(f"incomplete run: mode={mode} info={info!r} bestmove={bestmove!r}")
    fields = info.split()
    values = {
        "depth": fields[2],
        "score": " ".join(fields[4:6]),
        "nodes": fields[7],
        "time": fields[11],
    }
    return {
        "mode": mode, "position": position, "depth": int(values["depth"]),
        "score": values["score"], "nodes": int(values["nodes"]),
        "time_ms": int(values["time"]), "bestmove": bestmove.split()[1],
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, default=Path("target/release/sekirei"))
    parser.add_argument("--depth", type=int, default=4)
    parser.add_argument("--repeats", type=int, default=1)
    args = parser.parse_args()
    records = []
    for repeat in range(1, args.repeats + 1):
        for name, position in POSITIONS.items():
            for mode, threads in (("Speculative", 1), ("LazySMP", 2)):
                record = run(args.binary, mode, position, args.depth, threads)
                record["case"] = name
                record["repeat"] = repeat
                records.append(record)
    print(json.dumps({"schema": "sekirei.lazy-smp-fixed-depth.v1", "records": records}, indent=2))


if __name__ == "__main__":
    main()
