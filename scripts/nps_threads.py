#!/usr/bin/env python3
"""Thread-scaling measurement for a USI engine (Sekirei or YaneuraOu).

For each thread count, every position is searched once with ``go movetime``
and the last ``info`` line's depth and nodes are recorded.  The report gives
the median NPS, the median depth, and the NPS ratio against 1 thread, so the
multi-thread setting used for the final judgement (4 threads) can be compared
with the 1-thread figures every SPRT so far was run at.

Sekirei, Lazy SMP vs the default speculative mode::

    python3 scripts/nps_threads.py target/release/sekirei --evalfile nn.bin \\
        --threads 1,2,4 --option SearchMode=LazySMP
    python3 scripts/nps_threads.py target/release/sekirei --evalfile nn.bin \\
        --threads 1,2,4 --option SearchMode=Speculative --option SpecTopN=3

YaneuraOu (same positions, same evaluation file)::

    python3 scripts/nps_threads.py /path/YaneuraOu --yaneuraou --evalfile nn.bin \\
        --threads 1,2,4

Run on an otherwise idle machine; the thread counts must not exceed the
physical cores.  Outputs one JSON line per (threads, position) with
``--json`` for later comparison.  Results are speed diagnostics, not
playing-strength claims.
"""

from __future__ import annotations

import argparse
import json
import os
import statistics
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


class Usi:
    def __init__(self, binary: str, env: dict[str, str]):
        binary_path = Path(binary).resolve()
        self.proc = subprocess.Popen(
            [str(binary_path)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            bufsize=1,
            env=env,
            cwd=str(binary_path.parent),
        )

    def send(self, line: str) -> None:
        assert self.proc.stdin
        self.proc.stdin.write(line + "\n")
        self.proc.stdin.flush()

    def wait_for(self, token: str) -> list[str]:
        assert self.proc.stdout
        lines = []
        while True:
            line = self.proc.stdout.readline()
            if not line:
                raise RuntimeError("engine exited")
            line = line.rstrip("\n")
            lines.append(line)
            if line.startswith(token):
                return lines

    def quit(self) -> None:
        try:
            self.send("quit")
            self.proc.wait(timeout=5)
        except Exception:
            self.proc.kill()


def parse_info(lines: list[str]) -> tuple[int, int, int]:
    """(depth, nodes, nps) from the last info line that has nodes."""
    depth = nodes = nps = 0
    for line in lines:
        if not line.startswith("info") or " nodes " not in line:
            continue
        parts = line.split()
        for i, part in enumerate(parts[:-1]):
            if part == "depth":
                depth = int(parts[i + 1])
            elif part == "nodes":
                nodes = int(parts[i + 1])
            elif part == "nps":
                nps = int(parts[i + 1])
    return depth, nodes, nps


def engine_options(args: argparse.Namespace, threads: int) -> list[str]:
    if args.yaneuraou:
        evalfile = Path(args.evalfile).resolve()
        options = [
            f"EvalDir={evalfile.parent}",
            f"FV_SCALE={args.fv_scale}",
            f"Threads={threads}",
            f"USI_Hash={args.hash}",
            "USI_OwnBook=false",
            "BookFile=no_book",
            "NetworkDelay=0",
            "NetworkDelay2=0",
            "MinimumThinkingTime=0",
            "RoundUpToFullSecond=false",
        ]
    else:
        options = [
            f"FV_SCALE={args.fv_scale}",
            f"Threads={threads}",
            f"Hash={args.hash}",
            "UseBook=false",
        ]
        if args.evalfile is not None:
            options.insert(0, f"EvalFile={Path(args.evalfile).resolve()}")
    return options + list(args.option)


def read_positions(path: str, limit: int) -> list[str]:
    positions = []
    for line in Path(path).read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        positions.append(line if line.startswith("startpos") else f"sfen {line}")
        if len(positions) == limit:
            break
    return positions


def measure(args: argparse.Namespace, threads: int, positions: list[str]) -> list[dict]:
    env = dict(os.environ, RAYON_NUM_THREADS=str(threads))
    engine = Usi(args.engine, env)
    engine.send("usi")
    engine.wait_for("usiok")
    for option in engine_options(args, threads):
        name, _, value = option.partition("=")
        engine.send(f"setoption name {name} value {value}")
    engine.send("isready")
    engine.wait_for("readyok")
    rows = []
    try:
        for index, position in enumerate(positions):
            engine.send("usinewgame")
            engine.send(f"position {position}")
            start = time.perf_counter()
            engine.send(f"go movetime {args.movetime}")
            lines = engine.wait_for("bestmove")
            elapsed = time.perf_counter() - start
            depth, nodes, nps = parse_info(lines)
            if nps == 0 and elapsed > 0:
                nps = int(nodes / elapsed)
            rows.append(
                {
                    "threads": threads,
                    "position": index,
                    "depth": depth,
                    "nodes": nodes,
                    "nps": nps,
                    "elapsed_ms": round(elapsed * 1000),
                }
            )
            print(
                f"  threads {threads} pos {index:2d}: depth {depth:2d} "
                f"nodes {nodes:9d} nps {nps:8d}",
                file=sys.stderr,
                flush=True,
            )
    finally:
        engine.quit()
    return rows


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("engine", help="USI engine binary")
    parser.add_argument("--yaneuraou", action="store_true", help="engine is YaneuraOu")
    parser.add_argument(
        "--evalfile",
        help="HalfKP nn.bin; omit for a material-only Sekirei measurement",
    )
    parser.add_argument("--fv-scale", type=int, default=24)
    parser.add_argument("--hash", type=int, default=64)
    parser.add_argument("--threads", default="1,2,4", help="comma-separated thread counts")
    parser.add_argument("--movetime", type=int, default=2000, help="ms per position")
    parser.add_argument(
        "--positions", default=str(ROOT / "data/gate/openings_standard.sfen")
    )
    parser.add_argument("--count", type=int, default=20, help="positions to use")
    parser.add_argument(
        "--option", action="append", default=[], help="extra USI option Name=Value"
    )
    parser.add_argument("--json", help="write one JSON line per measurement")
    args = parser.parse_args()
    if args.yaneuraou and args.evalfile is None:
        parser.error("--yaneuraou requires --evalfile")

    positions = read_positions(args.positions, args.count)
    thread_counts = [int(t) for t in args.threads.split(",")]
    results: dict[int, list[dict]] = {}
    for threads in thread_counts:
        print(f"threads {threads}", file=sys.stderr, flush=True)
        results[threads] = measure(args, threads, positions)

    if args.json:
        with open(args.json, "w", encoding="utf-8") as out:
            for rows in results.values():
                for row in rows:
                    out.write(json.dumps(row) + "\n")

    base = statistics.median(r["nps"] for r in results[thread_counts[0]])
    label = Path(args.engine).name + (" " + " ".join(args.option) if args.option else "")
    print(f"{label}: {len(positions)} positions, {args.movetime} ms each")
    print("threads  median NPS   ratio  median depth  min..max depth")
    for threads in thread_counts:
        rows = results[threads]
        nps = statistics.median(r["nps"] for r in rows)
        depths = [r["depth"] for r in rows]
        print(
            f"{threads:7d}  {nps:10.0f}  {nps / base:6.2f}  "
            f"{statistics.median(depths):12.1f}  {min(depths)}..{max(depths)}"
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
