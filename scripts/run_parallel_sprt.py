#!/usr/bin/env python3
"""Play colour-reversed game pairs in parallel and stop on an SPRT verdict.

Each pair is one ``sekirei-match`` process with a single start position and
``--games-per-position 2``, so engine 1 plays both colours from the same
position. Up to ``--concurrency`` pairs run at once. Pair results are reduced
to win/draw/loss by their net score and fed to the same trinomial BayesElo
SPRT as ``run_ab_match.py --paired-sprt``.

Without ``--sprt`` the script plays exactly ``--pairs`` pairs and reports Elo
with a 95% interval, which is the fixed-gauge mode.

Self-play SPRT with the shared r6 weight::

    python3 scripts/run_parallel_sprt.py \\
        --engine1 cand/sekirei --engine2 base/sekirei \\
        --option1 EvalFile=/abs/nn_r6.bin --option1 FV_SCALE=24 \\
        --option2 EvalFile=/abs/nn_r6.bin --option2 FV_SCALE=24 \\
        --positions openings.sfen --byoyomi 200 \\
        --sprt 0,5 --pairs 5000 --out-dir runs/sprt-example

Options given with ``--option-both`` go to both engines. ``Threads=1`` and
``Hash=64`` are sent to both engines unless overridden. The default
concurrency is the number of performance cores minus one (macOS
``hw.perflevel0.physicalcpu``), or the CPU count minus one elsewhere.

Outputs in ``--out-dir``: ``pairs.jsonl`` (one line per finished pair, in
completion order), ``summary.json`` (rewritten after every pair), and each
pair's runner JSON and kifu under ``pairs/<index>/``.
"""

from __future__ import annotations

import argparse
import json
import os
import signal
import subprocess
import sys
import threading
import time
from concurrent.futures import FIRST_COMPLETED, Future, ThreadPoolExecutor, wait
from datetime import datetime, timezone
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from run_ab_match import elo, paired_outcome, sprt_bounds, sprt_llr  # noqa: E402
from sekirei_tools.artifacts import atomic_write_json, file_sha256  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
SCHEMA = "sekirei.parallel-sprt.v1"
DEFAULT_OPTIONS = ("Threads=1", "Hash=64")


def performance_cores() -> int | None:
    """Physical performance-core count on Apple silicon, else ``None``."""
    try:
        out = subprocess.run(
            ["sysctl", "-n", "hw.perflevel0.physicalcpu"],
            capture_output=True, text=True, timeout=5, check=True,
        ).stdout.strip()
        value = int(out)
        return value if value > 0 else None
    except (OSError, ValueError, subprocess.SubprocessError):
        return None


def default_concurrency() -> int:
    cores = performance_cores() or os.cpu_count() or 2
    return max(1, cores - 1)


def merge_options(specific: list[str], both: list[str]) -> list[str]:
    """Defaults, then ``--option-both``, then per-engine options; last name wins."""
    merged: dict[str, str] = {}
    for option in (*DEFAULT_OPTIONS, *both, *specific):
        name, sep, value = option.partition("=")
        if not sep or not name:
            raise ValueError(f"USI option must be Name=Value: {option!r}")
        merged.pop(name, None)
        merged[name] = value
    return [f"{name}={value}" for name, value in merged.items()]


def load_positions(path: Path) -> list[str]:
    positions = [
        line.strip() for line in path.read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    ]
    if not positions:
        raise ValueError(f"no start positions in {path}")
    return positions


def pair_score(summary: dict) -> float:
    """Engine 1's points over the pair (0, 0.5, ..., 2)."""
    return summary["engine1_wins"] + 0.5 * summary["draws"]


def validate_pair_summary(summary: dict) -> str | None:
    if summary.get("status") != "complete":
        return f"runner status {summary.get('status')!r}"
    if summary.get("games") != 2:
        return f"expected 2 games, got {summary.get('games')!r}"
    if summary.get("invalid_games"):
        return f"invalid games: {summary['invalid_games']}"
    return None


class Match:
    def __init__(self, args: argparse.Namespace, positions: list[str]):
        self.args = args
        self.positions = positions
        self.out_dir: Path = args.out_dir
        self.options1 = merge_options(args.option1, args.option_both)
        self.options2 = merge_options(args.option2, args.option_both)
        self.stop = threading.Event()
        self.lock = threading.Lock()
        self.procs: set[subprocess.Popen] = set()
        self.counts = {"win": 0, "draw": 0, "loss": 0}
        self.games = {"engine1_wins": 0, "draws": 0, "engine2_wins": 0}
        self.errors: list[dict] = []
        self.completed = 0
        self.verdict = "running"
        self.started = datetime.now(timezone.utc).isoformat()
        self.identities = {
            "engine1": file_sha256(args.engine1),
            "engine2": file_sha256(args.engine2),
            "match_binary": file_sha256(args.match_binary),
            "positions": file_sha256(args.positions),
        }
        self.lower, self.upper = (
            sprt_bounds(args.alpha, args.beta) if args.sprt else (None, None)
        )

    def command(self, index: int, pair_dir: Path, positions_file: Path) -> list[str]:
        cmd = [
            str(self.args.match_binary),
            "--engine1", str(self.args.engine1),
            "--engine2", str(self.args.engine2),
            "--positions", str(positions_file),
            "--games-per-position", "2",
            "--max-moves", str(self.args.max_moves),
            "--json", str(pair_dir / "result.json"),
            "--output", str(pair_dir / "kifu"),
        ]
        if self.args.time is not None:
            cmd += ["--time", str(self.args.time), "--inc", str(self.args.inc)]
        else:
            cmd += ["--byoyomi", str(self.args.byoyomi)]
        for option in self.options1:
            cmd += ["--engine-option1", option]
        for option in self.options2:
            cmd += ["--engine-option2", option]
        return cmd

    def play_pair(self, index: int) -> dict:
        position = self.positions[index % len(self.positions)]
        pair_dir = self.out_dir / "pairs" / f"{index:06d}"
        pair_dir.mkdir(parents=True, exist_ok=True)
        positions_file = pair_dir / "position.sfen"
        positions_file.write_text(position + "\n", encoding="utf-8")
        record: dict = {"pair": index, "position_index": index % len(self.positions)}
        if self.stop.is_set():
            record["error"] = "cancelled before start"
            return record
        log = (pair_dir / "runner.log").open("w", encoding="utf-8")
        proc = subprocess.Popen(
            self.command(index, pair_dir, positions_file),
            stdout=log, stderr=subprocess.STDOUT, start_new_session=True,
        )
        with self.lock:
            self.procs.add(proc)
        try:
            returncode = proc.wait()
        finally:
            with self.lock:
                self.procs.discard(proc)
            log.close()
        if self.stop.is_set() and returncode != 0:
            record["error"] = "cancelled"
            return record
        try:
            summary = json.loads((pair_dir / "result.json").read_text(encoding="utf-8"))
        except (OSError, ValueError) as error:
            record["error"] = f"exit {returncode}; unreadable result: {error}"
            return record
        problem = validate_pair_summary(summary)
        if problem or returncode != 0:
            record["error"] = problem or f"exit {returncode}"
            return record
        score = pair_score(summary)
        record.update(
            engine1_wins=summary["engine1_wins"],
            draws=summary["draws"],
            engine2_wins=summary["engine2_wins"],
            pair_score=score,
            outcome=paired_outcome(score, 0.0),
        )
        return record

    def record(self, record: dict) -> None:
        with (self.out_dir / "pairs.jsonl").open("a", encoding="utf-8") as handle:
            handle.write(json.dumps(record, sort_keys=True) + "\n")
        if "error" in record:
            if not record["error"].startswith("cancelled"):
                self.errors.append(record)
            return
        self.completed += 1
        self.counts[record["outcome"]] += 1
        for key in self.games:
            self.games[key] += record[key]

    def llr(self) -> float | None:
        if not self.args.sprt:
            return None
        elo0, elo1 = self.args.sprt
        return sprt_llr(self.counts["win"], self.counts["loss"], self.counts["draw"], elo0, elo1)

    def summary(self) -> dict:
        g = self.games
        game_elo, game_margin = elo(g["engine1_wins"], g["engine2_wins"], g["draws"])
        pair_elo, pair_margin = elo(self.counts["win"], self.counts["loss"], self.counts["draw"])
        llr = self.llr()
        return {
            "schema": SCHEMA,
            "started_at": self.started,
            "updated_at": datetime.now(timezone.utc).isoformat(),
            "verdict": self.verdict,
            "engine1": {"path": str(self.args.engine1), "sha256": self.identities["engine1"],
                        "options": self.options1},
            "engine2": {"path": str(self.args.engine2), "sha256": self.identities["engine2"],
                        "options": self.options2},
            "match_binary_sha256": self.identities["match_binary"],
            "positions": {"path": str(self.args.positions), "count": len(self.positions),
                          "sha256": self.identities["positions"]},
            "time_control": ({"time_ms": self.args.time, "inc_ms": self.args.inc}
                             if self.args.time is not None else {"byoyomi_ms": self.args.byoyomi}),
            "concurrency": self.args.concurrency,
            "pairs_limit": self.args.pairs,
            "pairs_completed": self.completed,
            "pair_counts": dict(self.counts),
            "game_counts": dict(g),
            "game_elo": round(game_elo, 2),
            "game_elo_ci95": round(game_margin, 2),
            "pair_elo": round(pair_elo, 2),
            "pair_elo_ci95": round(pair_margin, 2),
            "sprt": None if not self.args.sprt else {
                "elo0": self.args.sprt[0], "elo1": self.args.sprt[1],
                "alpha": self.args.alpha, "beta": self.args.beta,
                "scale": "BayesElo, trinomial over colour-reversed pairs",
                "llr": round(llr, 4), "lower": round(self.lower, 4), "upper": round(self.upper, 4),
            },
            "errors": len(self.errors),
        }

    def decide(self) -> str | None:
        if len(self.errors) > self.args.max_errors:
            return "aborted_errors"
        llr = self.llr()
        if llr is not None:
            if llr >= self.upper:
                return "H1_accepted"
            if llr <= self.lower:
                return "H0_accepted"
        return None

    def terminate_running(self) -> None:
        with self.lock:
            procs = list(self.procs)
        for proc in procs:
            try:
                os.killpg(proc.pid, signal.SIGTERM)
            except (ProcessLookupError, PermissionError):
                pass
        deadline = time.monotonic() + 5
        for proc in procs:
            try:
                proc.wait(timeout=max(0.1, deadline - time.monotonic()))
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(proc.pid, signal.SIGKILL)
                except (ProcessLookupError, PermissionError):
                    pass

    def run(self) -> str:
        self.out_dir.mkdir(parents=True, exist_ok=True)
        atomic_write_json(self.out_dir / "summary.json", self.summary())
        next_index = 0
        running: dict[Future, int] = {}
        with ThreadPoolExecutor(max_workers=self.args.concurrency) as pool:
            try:
                while True:
                    while (not self.stop.is_set() and next_index < self.args.pairs
                           and len(running) < self.args.concurrency):
                        running[pool.submit(self.play_pair, next_index)] = next_index
                        next_index += 1
                    if not running:
                        break
                    done, _ = wait(running, return_when=FIRST_COMPLETED)
                    for future in done:
                        running.pop(future)
                        self.record(future.result())
                    if not self.stop.is_set():
                        decision = self.decide()
                        if decision:
                            self.verdict = decision
                            self.stop.set()
                            self.terminate_running()
                    atomic_write_json(self.out_dir / "summary.json", self.summary())
                    self.progress()
            except KeyboardInterrupt:
                self.verdict = "interrupted"
                self.stop.set()
                self.terminate_running()
        if self.verdict == "running":
            self.verdict = "pairs_limit_reached" if self.args.sprt else "complete"
        atomic_write_json(self.out_dir / "summary.json", self.summary())
        self.progress(final=True)
        return self.verdict

    def progress(self, final: bool = False) -> None:
        s = self.summary()
        llr = f" LLR {s['sprt']['llr']:+.2f} [{s['sprt']['lower']:.2f}, {s['sprt']['upper']:.2f}]" \
            if s["sprt"] else ""
        prefix = "final" if final else "pairs"
        print(f"{prefix} {self.completed}/{self.args.pairs} W/D/L {self.counts['win']}/"
              f"{self.counts['draw']}/{self.counts['loss']} games {s['game_counts']} "
              f"Elo {s['game_elo']:+.1f}±{s['game_elo_ci95']:.1f}{llr} "
              f"errors {len(self.errors)} {self.verdict}", flush=True)


def parse_sprt(text: str) -> tuple[float, float]:
    try:
        elo0, elo1 = (float(part) for part in text.split(","))
    except ValueError as error:
        raise argparse.ArgumentTypeError("--sprt takes ELO0,ELO1") from error
    if elo1 <= elo0:
        raise argparse.ArgumentTypeError("--sprt needs ELO1 > ELO0")
    return elo0, elo1


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--engine1", type=Path, required=True, help="engine under test (reported side)")
    parser.add_argument("--engine2", type=Path, required=True, help="baseline or reference engine")
    parser.add_argument("--option1", action="append", default=[], metavar="NAME=VALUE",
                        help="USI option for engine 1 (repeatable)")
    parser.add_argument("--option2", action="append", default=[], metavar="NAME=VALUE",
                        help="USI option for engine 2 (repeatable)")
    parser.add_argument("--option-both", action="append", default=[], metavar="NAME=VALUE",
                        help="USI option for both engines (repeatable)")
    parser.add_argument("--positions", type=Path, required=True,
                        help="start positions, one per line; pair i uses line i modulo the count")
    parser.add_argument("--pairs", type=int, required=True, help="maximum (or, without --sprt, exact) pair count")
    parser.add_argument("--byoyomi", type=int, default=1000, help="ms per move (default 1000)")
    parser.add_argument("--time", type=int, help="Fischer initial ms per side (replaces --byoyomi)")
    parser.add_argument("--inc", type=int, default=0, help="Fischer increment ms (with --time)")
    parser.add_argument("--max-moves", type=int, default=320)
    parser.add_argument("--sprt", type=parse_sprt, metavar="ELO0,ELO1",
                        help="BayesElo hypotheses; stop when H0 or H1 is accepted")
    parser.add_argument("--alpha", type=float, default=0.05)
    parser.add_argument("--beta", type=float, default=0.05)
    parser.add_argument("--concurrency", type=int, default=None,
                        help="parallel pairs (default: performance cores - 1)")
    parser.add_argument("--max-errors", type=int, default=3,
                        help="abort after more than this many failed pairs (default 3)")
    parser.add_argument("--match-binary", type=Path, default=ROOT / "target/release/sekirei-match")
    parser.add_argument("--out-dir", type=Path, required=True, help="new or empty output directory")
    args = parser.parse_args(argv)
    if args.concurrency is None:
        args.concurrency = default_concurrency()
    if args.concurrency < 1 or args.pairs < 1:
        parser.error("--concurrency and --pairs must be positive")
    if not 0 < args.alpha < 1 or not 0 < args.beta < 1:
        parser.error("--alpha and --beta must be in (0, 1)")
    try:
        merge_options(args.option1, args.option_both)
        merge_options(args.option2, args.option_both)
    except ValueError as error:
        parser.error(str(error))
    if args.out_dir.exists() and any(args.out_dir.iterdir()):
        parser.error(f"--out-dir is not empty: {args.out_dir}")
    for name in ("engine1", "engine2", "match_binary", "positions"):
        if not getattr(args, name).is_file():
            parser.error(f"--{name.replace('_', '-')} not found: {getattr(args, name)}")
    return args


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    positions = load_positions(args.positions)
    verdict = Match(args, positions).run()
    return 1 if verdict in ("aborted_errors", "interrupted") else 0


if __name__ == "__main__":
    sys.exit(main())
