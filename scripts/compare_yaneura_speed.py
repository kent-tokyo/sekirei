#!/usr/bin/env python3
"""Bounded ABBA NPS diagnostics using a shared explicit HalfKP evaluator.

Cross-engine NPS counts different search trees; this measures reported node
throughput, not strength or useful work. Depth is descriptive, since depth
semantics differ between engines. Do not select tuning or defaults using an
external shared evaluator from this diagnostic.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import platform
import signal
import statistics
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

import nps_threads

DRIVER = Path(__file__).resolve().with_name("nps_threads.py")
ABBA = ("reference", "sekirei", "sekirei", "reference")
SEKIREI_OPTIONS = (
    "SearchMode=Auto", "SpecTopN=0", "NnueOutput=absolute", "LazyDiagnostics=false",
    "MoveOverhead=0",
)
LIMITATIONS = [
    "Speed diagnostic only: reported nodes count different search trees.",
    "Depth is descriptive; cross-engine depth does not measure equivalent work.",
    "This is not a playing-strength or useful-work result.",
    "Do not choose tuning, search defaults, training labels, or candidates using this external-evaluator lane.",
]


def timestamp() -> str:
    return datetime.now(timezone.utc).isoformat()


def artifact_snapshot(paths: dict[str, Path]) -> dict:
    return {
        name: {"path": str(path), "sha256": nps_threads.sha256_file(path)}
        for name, path in paths.items()
    }


def driver_args(args: argparse.Namespace, lane: str) -> argparse.Namespace:
    return argparse.Namespace(
        evalfile=str(args.evalfile),
        yaneuraou=lane == "reference",
        fv_scale=24,
        hash=64,
        lazy_diagnostics=False,
        option=list(SEKIREI_OPTIONS) if lane == "sekirei" else [],
    )


def command(args: argparse.Namespace, lane: str, threads: int, output: Path) -> list[str]:
    binary = args.engine if lane == "sekirei" else args.reference
    result = [
        sys.executable, str(DRIVER), str(binary),
        "--evalfile", str(args.evalfile),
        "--positions", str(args.positions),
        "--threads", str(threads), "--count", str(args.count),
        "--movetime", str(args.movetime), "--fv-scale", "24", "--hash", "64",
        "--json", str(output),
    ]
    if lane == "reference":
        result.append("--yaneuraou")
    else:
        for option in SEKIREI_OPTIONS:
            result.extend(["--option", option])
    return result


def run_bounded(cmd: list[str], stdout: Path, stderr: Path, timeout: float) -> int:
    """Kill and reap only this invocation's session on timeout, retaining logs."""
    with stdout.open("wb") as out, stderr.open("wb") as err:
        process = subprocess.Popen(
            cmd, stdout=out, stderr=err, start_new_session=True
        )
        try:
            return process.wait(timeout=timeout)
        except (subprocess.TimeoutExpired, KeyboardInterrupt):
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            finally:
                process.wait()
            raise


def validate_rows(rows: list[dict], args: argparse.Namespace, lane: str,
                  threads: int, artifacts: dict, positions: list[str]) -> list[dict]:
    if len(rows) != args.count:
        raise ValueError(f"expected {args.count} rows, got {len(rows)}")
    binary_key = "engine" if lane == "sekirei" else "reference"
    expected = {
        "engine": artifacts[binary_key]["path"],
        "engine_sha256": artifacts[binary_key]["sha256"],
        "positions": artifacts["positions"]["path"],
        "positions_file_sha256": artifacts["positions"]["sha256"],
        "effective_positions_sha256": nps_threads.effective_positions_sha256(positions),
        "effective_position_count": args.count,
        "evaluator": {"kind": "halfkp", **artifacts["evalfile"]},
        "counter_overhead_included": False,
        "options": nps_threads.engine_options(driver_args(args, lane), threads),
        "budget": f"go movetime {args.movetime}",
    }
    seen = set()
    for row in rows:
        if row.get("schema") != "sekirei.lazy-smp-scaling-row.v2":
            raise ValueError("unexpected row schema")
        index = row.get("position")
        if type(index) is not int or index not in range(args.count) or index in seen:
            raise ValueError(f"missing, duplicate, or invalid position: {index!r}")
        seen.add(index)
        if row.get("threads") != threads or row.get("repeat") != 0:
            raise ValueError("unexpected threads or repeat")
        # Ordinary Lazy SMP emits a post-search worker summary even when
        # LazyDiagnostics is false. Only the opt-in TT/overlap/root-hint
        # records and nonzero TT counters imply instrumented searches.
        if any(key.startswith("lazy_smp") and key != "lazy_smp" for key in row):
            raise ValueError("diagnostic counters must be disabled")
        if "lazy_smp" in row:
            summary = row["lazy_smp"]
            if lane != "sekirei" or not isinstance(summary, dict):
                raise ValueError("unexpected Lazy SMP summary")
            if (type(summary.get("selected_worker")) is not int
                    or type(summary.get("workers")) is not int
                    or summary["workers"] != threads
                    or not 0 <= summary["selected_worker"] < threads):
                raise ValueError("invalid Lazy SMP worker summary")
            for key, value in summary.items():
                if key.startswith("selected_tt_") and value != 0:
                    raise ValueError("diagnostic counters must be disabled")
            workers = summary.get("worker_stats", [])
            if not isinstance(workers, list) or any(not isinstance(worker, dict) for worker in workers):
                raise ValueError("invalid Lazy SMP worker statistics")
            for worker in workers:
                if any(value != 0 for key, value in worker.items() if key.startswith("tt_")):
                    raise ValueError("diagnostic counters must be disabled")
        for key in ("nps", "nodes", "depth", "elapsed_ms"):
            value = row.get(key)
            minimum = 0 if key == "depth" else 1
            if type(value) is not int or value < minimum:
                raise ValueError(f"invalid {key}: {value!r}")
        if not isinstance(row.get("bestmove"), str) or not row["bestmove"]:
            raise ValueError("missing bestmove")
        provenance = row.get("provenance", {})
        for key, value in expected.items():
            if provenance.get(key) != value or (
                key == "counter_overhead_included"
                and provenance.get(key) is not False
            ):
                raise ValueError(f"mismatched provenance {key}")
    return sorted(rows, key=lambda row: row["position"])


def summarize(arms: list[dict], count: int) -> list[dict]:
    results = []
    for threads in dict.fromkeys(arm["threads"] for arm in arms):
        selected = [arm for arm in arms if arm["threads"] == threads]
        if len(selected) != 4 or any(arm["status"] != "complete" for arm in selected):
            continue
        if tuple(arm["lane"] for arm in selected) != ABBA:
            raise ValueError("complete thread block must follow reference/Sekirei/Sekirei/reference")
        ratios = []
        for position in range(count):
            means = {
                lane: statistics.mean(
                    row["nps"] for arm in selected if arm["lane"] == lane
                    for row in arm["rows"] if row["position"] == position
                )
                for lane in ("sekirei", "reference")
            }
            ratios.append({"position": position,
                           "sekirei_over_reference": means["sekirei"] / means["reference"]})
        results.append({
            "threads": threads,
            "arms": [{"lane": arm["lane"], "index": arm["index"],
                      "median_nps": statistics.median(row["nps"] for row in arm["rows"]),
                      "median_depth_descriptive": statistics.median(row["depth"] for row in arm["rows"])}
                     for arm in selected],
            "per_position_mean_nps_ratios": ratios,
            "geomean_sekirei_over_reference": math.exp(statistics.mean(
                math.log(row["sekirei_over_reference"]) for row in ratios)),
        })
    return results


def write_report(path: Path, report: dict) -> None:
    temporary = path.with_suffix(".tmp")
    temporary.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    temporary.replace(path)


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    for name in ("engine", "reference", "evalfile", "positions", "output-directory"):
        parser.add_argument(f"--{name}", type=Path, required=True)
    parser.add_argument("--threads", default="1,2,4,8")
    parser.add_argument("--count", type=int, default=4)
    parser.add_argument("--movetime", type=int, default=1000)
    parser.add_argument("--timeout", type=float, help="whole-arm timeout in seconds; default search budget + 120s")
    args = parser.parse_args(argv)
    try:
        args.thread_counts = [int(value) for value in args.threads.split(",")]
    except ValueError:
        parser.error("--threads must contain comma-separated integers")
    if not args.thread_counts or min(args.thread_counts) < 1 or len(set(args.thread_counts)) != len(args.thread_counts):
        parser.error("--threads must contain unique positive integers")
    if args.count < 1 or args.movetime < 1:
        parser.error("--count and --movetime must be positive")
    if args.timeout is None:
        args.timeout = args.count * args.movetime / 1000 + 120
    if not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error("--timeout must be finite and positive")
    for name in ("engine", "reference", "evalfile", "positions", "output_directory"):
        setattr(args, name, getattr(args, name).resolve())
    if args.evalfile.name != "nn.bin":
        parser.error("--evalfile must be named nn.bin: YaneuraOu uses EvalDir/nn.bin")
    return args


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    paths = {key: getattr(args, key) for key in ("engine", "reference", "evalfile", "positions")}
    paths["driver"] = DRIVER
    positions = nps_threads.read_positions(str(args.positions), args.count)
    if len(positions) != args.count:
        raise ValueError("positions file contains fewer positions than --count")
    initial = artifact_snapshot(paths)
    args.output_directory.mkdir(parents=True, exist_ok=False)
    report_path = args.output_directory / "report.json"
    report = {
        "schema": "sekirei.yaneura-speed-abba.v1", "status": "running",
        "started_at_utc": timestamp(), "limitations": LIMITATIONS,
        "host": {"system": platform.system(), "release": platform.release(),
                 "machine": platform.machine(), "processor": platform.processor()},
        "config": {"count": args.count, "movetime_ms": args.movetime,
                   "threads": args.thread_counts, "timeout_seconds": args.timeout,
                   "hash_mib": 64, "fv_scale": 24, "abba": list(ABBA)},
        "artifacts_before": initial, "arms": [], "summary": [],
    }
    write_report(report_path, report)
    try:
        for threads in args.thread_counts:
            for lane in ABBA:
                index = len(report["arms"])
                stem = f"{index:02d}-t{threads}-{lane}"
                rows_path = args.output_directory / f"{stem}.jsonl"
                arm = {"index": index, "threads": threads, "lane": lane,
                       "status": "running", "started_at_utc": timestamp(),
                       "rows_path": str(rows_path), "artifacts_before": artifact_snapshot(paths)}
                report["arms"].append(arm)
                arm["command"] = command(args, lane, threads, rows_path)
                arm["stdout"] = str(args.output_directory / f"{stem}.stdout.log")
                arm["stderr"] = str(args.output_directory / f"{stem}.stderr.log")
                write_report(report_path, report)
                try:
                    if arm["artifacts_before"] != initial:
                        raise ValueError("input artifact changed before arm")
                    arm["returncode"] = run_bounded(arm["command"], Path(arm["stdout"]),
                                                    Path(arm["stderr"]), args.timeout)
                    if arm["returncode"] != 0:
                        raise ValueError(f"driver exited with code {arm['returncode']}")
                    rows = [json.loads(line) for line in rows_path.read_text(encoding="utf-8").splitlines() if line.strip()]
                    arm["rows"] = validate_rows(rows, args, lane, threads, initial, positions)
                    arm["status"] = "complete"
                except BaseException as error:
                    arm["status"] = "failed"
                    arm["error"] = f"{type(error).__name__}: {error}"
                    raise
                finally:
                    arm["finished_at_utc"] = timestamp()
                    arm["artifacts_after"] = artifact_snapshot(paths)
                    if arm["artifacts_after"] != initial:
                        arm["status"] = "failed"
                        arm["error"] = "input artifact changed during arm"
                    report["summary"] = summarize(report["arms"], args.count)
                    write_report(report_path, report)
                if arm["status"] != "complete":
                    raise ValueError(arm["error"])
        report["status"] = "complete"
    except BaseException as error:
        report["status"] = "failed"
        report["error"] = f"{type(error).__name__}: {error}"
        raise
    finally:
        report["finished_at_utc"] = timestamp()
        report["artifacts_after"] = artifact_snapshot(paths)
        write_report(report_path, report)
    print(report_path)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
