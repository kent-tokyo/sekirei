#!/usr/bin/env python3
"""Thread-scaling measurement for a USI engine (Sekirei or YaneuraOu).

For each thread count, every position is searched with ``go movetime``
and the last ``info`` line's depth and nodes are recorded.  The report gives
the median NPS, the median depth, and the NPS ratio against 1 thread, so the
multi-thread setting used for the final judgement (4 threads) can be compared
with the 1-thread figures every SPRT so far was run at.

``--lazy-diagnostics`` enables Sekirei's opt-in counter path and records
worker-local TT probes/hits/stores plus shared-table write collisions.  This
adds relaxed atomic counter traffic, so use a separate run without it for NPS.
Use ``--repeats`` to measure root-bestmove stability on the same positions.

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


def parse_lazy_smp(lines: list[str]) -> dict[str, int] | None:
    """Selected-worker diagnostics from the last Lazy SMP info line."""
    prefix = "info string lazy_smp "
    for line in reversed(lines):
        if not line.startswith(prefix):
            continue
        parts = line[len(prefix) :].split()
        try:
            selected = int(parts[parts.index("selected") + 1].removeprefix("w"))
            node_share_permille = int(parts[parts.index("node_share_permille") + 1])
            agreement, workers = parts[parts.index("move_agreement") + 1].split("/", 1)
            stop_lag_ms = int(parts[parts.index("stop_lag_ms") + 1])
            result = {
                "selected_worker": selected,
                "selected_node_share_permille": node_share_permille,
                "move_agreement": int(agreement),
                "workers": int(workers),
                "stop_lag_ms": stop_lag_ms,
            }
            summary = parts[-1]
            worker_stats = parse_lazy_workers(summary)
            if worker_stats:
                result["worker_stats"] = worker_stats
            return result
        except (ValueError, IndexError):
            return None
    return None


def parse_lazy_workers(summary: str) -> list[dict[str, int | str | bool]]:
    """Parse the compact comma-separated Lazy SMP worker summary."""
    workers = []
    for raw_worker in summary.split(","):
        fields = raw_worker.split(":")
        if len(fields) != 10 or not fields[0].startswith("w"):
            return []
        worker_label = fields[0]
        selected = worker_label.endswith("*")
        try:
            workers.append(
                {
                    "worker": int(worker_label[1:].removesuffix("*")),
                    "selected": selected,
                    "depth": int(fields[1].removeprefix("d")),
                    "nodes": int(fields[2].removeprefix("n")),
                    "score": int(fields[3].removeprefix("s")),
                    "elapsed_ms": int(fields[4].removeprefix("t")),
                    "aborted": fields[5].removeprefix("x") == "1",
                    "abort_reason": fields[6].removeprefix("a"),
                    "tt_probes": int(fields[7].removeprefix("p")),
                    "tt_hits": int(fields[8].removeprefix("h")),
                    "tt_stores": int(fields[9].removeprefix("r")),
                }
            )
        except ValueError:
            return []
    return workers


def parse_lazy_smp_tt(lines: list[str]) -> dict[str, int] | None:
    """Aggregate TT diagnostics from the last Lazy SMP TT info line."""
    prefix = "info string lazy_smp_tt "
    keys = (
        "probes",
        "hits",
        "attempted",
        "committed",
        "same_hash",
        "equal_depth",
        "shallower_rejected",
        "collision_overwrites",
    )
    for line in reversed(lines):
        if not line.startswith(prefix):
            continue
        parts = line[len(prefix) :].split()
        try:
            return {key: int(parts[parts.index(key) + 1]) for key in keys}
        except (ValueError, IndexError):
            return None
    return None


def parse_bestmove(lines: list[str]) -> str:
    for line in reversed(lines):
        if line.startswith("bestmove "):
            return line.split(maxsplit=2)[1]
    return ""


def go_command(args: argparse.Namespace) -> str:
    if args.depth is not None:
        return f"go depth {args.depth}"
    return f"go movetime {args.movetime}"


def summarize_lazy_tt(rows: list[dict]) -> dict[str, float | int] | None:
    diagnostics = [row.get("lazy_smp_tt") for row in rows]
    diagnostics = [item for item in diagnostics if item is not None]
    if not diagnostics:
        return None
    totals = {
        key: sum(item[key] for item in diagnostics)
        for key in (
            "probes",
            "hits",
            "attempted",
            "committed",
            "same_hash",
            "equal_depth",
            "shallower_rejected",
            "collision_overwrites",
        )
    }
    totals["hit_rate"] = totals["hits"] / max(totals["probes"], 1)
    totals["same_hash_rate"] = totals["same_hash"] / max(totals["attempted"], 1)
    totals["collision_rate"] = totals["collision_overwrites"] / max(
        totals["attempted"], 1
    )
    return totals


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
        if args.lazy_diagnostics:
            options.append("LazyDiagnostics=true")
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
            for repeat in range(args.repeats):
                engine.send("usinewgame")
                engine.send(f"position {position}")
                start = time.perf_counter()
                engine.send(go_command(args))
                lines = engine.wait_for("bestmove")
                elapsed = time.perf_counter() - start
                depth, nodes, nps = parse_info(lines)
                lazy_smp = parse_lazy_smp(lines)
                lazy_smp_tt = parse_lazy_smp_tt(lines)
                if nps == 0 and elapsed > 0:
                    nps = int(nodes / elapsed)
                row = {
                    "threads": threads,
                    "position": index,
                    "repeat": repeat,
                    "bestmove": parse_bestmove(lines),
                    "depth": depth,
                    "nodes": nodes,
                    "nps": nps,
                    "elapsed_ms": round(elapsed * 1000),
                }
                if lazy_smp is not None:
                    row["lazy_smp"] = lazy_smp
                if lazy_smp_tt is not None:
                    row["lazy_smp_tt"] = lazy_smp_tt
                rows.append(row)
                lazy_suffix = ""
                if lazy_smp is not None:
                    lazy_suffix = (
                        f" selected w{lazy_smp['selected_worker']}"
                        f" share {lazy_smp['selected_node_share_permille'] / 10:.1f}%"
                        f" agree {lazy_smp['move_agreement']}/{lazy_smp['workers']}"
                        f" lag {lazy_smp['stop_lag_ms']}ms"
                    )
                if lazy_smp_tt is not None:
                    hit_rate = lazy_smp_tt["hits"] / max(lazy_smp_tt["probes"], 1)
                    lazy_suffix += (
                        f" tt-hit {hit_rate:.1%}"
                        f" collisions {lazy_smp_tt['collision_overwrites']}"
                    )
                print(
                    f"  threads {threads} pos {index:2d} run {repeat + 1}:"
                    f" depth {depth:2d} nodes {nodes:9d} nps {nps:8d}"
                    f" bestmove {row['bestmove']}{lazy_suffix}",
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
    budget = parser.add_mutually_exclusive_group()
    budget.add_argument("--movetime", type=int, default=2000, help="ms per position")
    budget.add_argument("--depth", type=int, help="fixed depth instead of wall time")
    parser.add_argument(
        "--positions", default=str(ROOT / "data/gate/openings_standard.sfen")
    )
    parser.add_argument("--count", type=int, default=20, help="positions to use")
    parser.add_argument(
        "--repeats", type=int, default=1, help="search each position this many times"
    )
    parser.add_argument(
        "--lazy-diagnostics",
        action="store_true",
        help="enable opt-in Sekirei Lazy SMP TT counters (adds overhead)",
    )
    parser.add_argument(
        "--option", action="append", default=[], help="extra USI option Name=Value"
    )
    parser.add_argument("--json", help="write one JSON line per measurement")
    args = parser.parse_args()
    if args.yaneuraou and args.evalfile is None:
        parser.error("--yaneuraou requires --evalfile")
    if args.yaneuraou and args.lazy_diagnostics:
        parser.error("--lazy-diagnostics is Sekirei-only")
    if args.repeats < 1:
        parser.error("--repeats must be at least 1")
    if args.depth is not None and args.depth < 1:
        parser.error("--depth must be at least 1")

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
    budget_label = (
        f"fixed depth {args.depth}"
        if args.depth is not None
        else f"{args.movetime} ms each"
    )
    print(f"{label}: {len(positions)} positions x {args.repeats} repeats, {budget_label}")
    print("threads  median NPS   ratio  median depth  min..max depth  bestmove stability")
    for threads in thread_counts:
        rows = results[threads]
        nps = statistics.median(r["nps"] for r in rows)
        depths = [r["depth"] for r in rows]
        stable = []
        for position in range(len(positions)):
            moves = [r["bestmove"] for r in rows if r["position"] == position]
            stable.append(max(moves.count(move) for move in set(moves)) / len(moves))
        print(
            f"{threads:7d}  {nps:10.0f}  {nps / base:6.2f}  "
            f"{statistics.median(depths):12.1f}  {min(depths)}..{max(depths)}"
            f"  {statistics.mean(stable):17.1%}"
        )
    tt_summaries = {
        threads: summarize_lazy_tt(results[threads]) for threads in thread_counts
    }
    if any(summary is not None for summary in tt_summaries.values()):
        print("TT diagnostic lane (counter overhead included; do not use as timing baseline)")
        print("threads  hit rate  same-hash writes  collision writes")
        for threads in thread_counts:
            summary = tt_summaries[threads]
            if summary is None:
                continue
            print(
                f"{threads:7d}  {summary['hit_rate']:8.1%}"
                f"  {summary['same_hash_rate']:16.1%}"
                f"  {summary['collision_rate']:15.3%}"
            )
    return 0


if __name__ == "__main__":
    sys.exit(main())
