#!/usr/bin/env python3
"""Freeze artifacts and compare one search factor in per-position A/B/B/A blocks.

Fresh engine processes isolate every search. Diagnostic counters and counter-free
wall-time runs are distinct lanes; this script does not estimate Elo or adopt a
candidate. The output directory must be new so interrupted runs cannot be pooled.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import platform
import shutil
import subprocess
import statistics
import time
from datetime import datetime, timezone
from pathlib import Path

from nps_threads import positive_u64, ASPIRATION_KEYS, effective_positions_sha256, measure, option_value, summarize_lazy_root_hints


def schedule(threads: list[int], positions: int, repeats: int):
    for repeat in range(repeats):
        for count in threads:
            for position in range(positions):
                for slot, side in enumerate("ABBA"):
                    yield repeat, count, position, slot, side


def side_changes(a: list[str], b: list[str]) -> dict[str, tuple[str | None, str | None]]:
    def mapping(options):
        result = {}
        for option in options:
            key, separator, value = option.partition("=")
            if not separator or not key or key in result:
                raise ValueError("side options must be unique Name=Value entries")
            result[key] = value
        return result
    left, right = mapping(a), mapping(b)
    changes = {key: (left.get(key), right.get(key)) for key in left.keys() | right.keys()
               if left.get(key) != right.get(key)}
    if len(changes) > 1:
        raise ValueError("one-factor lane permits only one changed side option")
    if "LazyFlags" in changes:
        first, second = changes["LazyFlags"]
        if first is None or second is None:
            raise ValueError("set LazyFlags explicitly on both sides")
        delta = int(first) ^ int(second)
        if delta == 0 or delta & (delta - 1):
            raise ValueError("LazyFlags candidate must change exactly one bit")
    return changes


def host_activity(strict: bool = False) -> dict:
    """Record a read-only process snapshot; never stop another task's process."""
    try:
        result = subprocess.run(["ps", "-Ao", "pid,pcpu,comm"], capture_output=True,
                                text=True, check=True, timeout=10)
        if strict:
            lines = result.stdout.splitlines()
            if not lines or lines[0].split() not in (["PID", "%CPU", "COMM"], ["PID", "%CPU", "COMMAND"]):
                raise ValueError("unsupported process snapshot header")
            processes = []
            for line in lines[1:]:
                if not line.strip():
                    continue
                fields = line.strip().split(None, 2)
                if len(fields) != 3:
                    raise ValueError("malformed process snapshot row")
                pid, cpu, command = int(fields[0]), float(fields[1]), fields[2]
                if pid < 1 or not math.isfinite(cpu) or cpu < 0 or not command:
                    raise ValueError("invalid process snapshot values")
                processes.append({"pid": pid, "cpu_percent": cpu, "command": command})
            if not processes:
                raise ValueError("empty process snapshot")
            return {"available": True, "busy_process_threshold_percent": 25,
                    "busy_processes": [process for process in processes
                                       if process["pid"] != os.getpid() and process["cpu_percent"] >= 25],
                    "load_average": list(os.getloadavg())}
        active = []
        for line in result.stdout.splitlines()[1:]:
            fields = line.strip().split(None, 2)
            if len(fields) != 3:
                continue
            pid, cpu, command = fields
            if int(pid) != os.getpid() and float(cpu) >= 25:
                active.append({"pid": int(pid), "cpu_percent": float(cpu), "command": command})
        return {"available": True, "busy_process_threshold_percent": 25,
                "busy_processes": active, "load_average": list(os.getloadavg())}
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        return {"available": False, "error": str(error), "busy_processes": None}


def freeze(source: Path, destination: Path) -> str:
    digest = hashlib.sha256()
    with source.open("rb") as original, destination.open("xb") as saved:
        for chunk in iter(lambda: original.read(1024 * 1024), b""):
            digest.update(chunk)
            saved.write(chunk)
    shutil.copymode(source, destination)
    return digest.hexdigest()


def idle_snapshot_state(snapshot) -> str:
    """Only complete, finite snapshots from host_activity can admit a run."""
    if not isinstance(snapshot, dict) or snapshot.get("available") is not True:
        return "unknown"
    if snapshot.get("busy_process_threshold_percent") != 25:
        return "unknown"
    load = snapshot.get("load_average")
    processes = snapshot.get("busy_processes")
    if (not isinstance(load, list) or len(load) != 3
            or any(isinstance(value, bool) or not isinstance(value, (int, float))
                   or not math.isfinite(value) or value < 0 for value in load)
            or not isinstance(processes, list)):
        return "unknown"
    for process in processes:
        if not isinstance(process, dict):
            return "unknown"
        pid, cpu, command = (process.get(key) for key in ("pid", "cpu_percent", "command"))
        if (type(pid) is not int or pid < 1 or isinstance(cpu, bool)
                or not isinstance(cpu, (int, float)) or not math.isfinite(cpu)
                or cpu < 25 or not isinstance(command, str) or not command):
            return "unknown"
    return "busy" if processes else "idle"


def capacity_snapshot(output: Path, minimum_free_mib: int) -> dict:
    """Sample this output filesystem; this is neither a reservation nor isolation."""
    snapshot = {"path": str(output.resolve()), "minimum_free_bytes": minimum_free_mib * 1024**2}
    try:
        free = shutil.disk_usage(output).free
        if type(free) is not int or free < 0:
            raise ValueError("invalid free disk byte count")
        return {**snapshot, "available": True, "free_bytes": free,
                "floor_pass": free >= snapshot["minimum_free_bytes"]}
    except (OSError, ValueError, TypeError, AttributeError) as error:
        return {**snapshot, "available": False, "free_bytes": None,
                "floor_pass": False, "error": str(error)}


def wait_for_idle(manifest: dict, save_manifest, initial_activity: dict,
                  started: float, request: dict, output: Path | None = None) -> bool:
    """Bounded preflight admission, not continuous host isolation."""
    admission = manifest["idle_admission"]
    deadline = started + request["wait_seconds"]
    activity = initial_activity
    consecutive = 0
    qualified_consecutive = 0
    minimum_free_mib = request.get("minimum_free_mib", 0)
    if minimum_free_mib and output is None:
        raise ValueError("capacity admission requires an output filesystem")
    timeout_status = "blocked_host_resources" if minimum_free_mib else "blocked_host_busy"
    timeout_error = ("idle/capacity admission timed out; no processes stopped" if minimum_free_mib
                     else "idle admission timed out; no processes stopped")
    while True:
        elapsed = time.monotonic() - started
        saved_activity, state = record_idle_snapshot(activity)
        consecutive = consecutive + 1 if state == "idle" else 0
        sample = {"sample_index": len(admission["samples"]),
                  "observed_at_utc": datetime.now(timezone.utc).isoformat(),
                  "elapsed_seconds": elapsed, "state": state,
                  "consecutive_idle_samples": consecutive, "host_activity": saved_activity}
        qualified = state == "idle"
        if minimum_free_mib:
            capacity = capacity_snapshot(output, minimum_free_mib)
            qualified = qualified and capacity["floor_pass"]
            qualified_consecutive = qualified_consecutive + 1 if qualified else 0
            sample.update(capacity=capacity, qualified=qualified,
                          consecutive_qualified_samples=qualified_consecutive)
            admission["consecutive_qualified_samples"] = qualified_consecutive
        else:
            qualified_consecutive = consecutive
        admission["samples"].append(sample)
        admission.update(elapsed_seconds=elapsed, consecutive_idle_samples=consecutive)
        if time.monotonic() > deadline:
            admission["status"] = "timeout"
        elif qualified_consecutive >= request["stable_samples"]:
            admission["status"] = "admitted"
            manifest["host_activity_before"] = saved_activity
            save_manifest()
            return True
        elif time.monotonic() >= deadline:
            admission["status"] = "timeout"
        if admission["status"] == "timeout":
            manifest.update(status=timeout_status, timing_qualified=False, error=timeout_error)
            save_manifest()
            return False
        save_manifest()
        remaining = deadline - time.monotonic()
        if remaining < request["poll_seconds"]:
            time.sleep(max(0, remaining))
            admission.update(status="timeout", elapsed_seconds=time.monotonic() - started)
            manifest.update(status=timeout_status, timing_qualified=False, error=timeout_error)
            save_manifest()
            return False
        time.sleep(request["poll_seconds"])
        if time.monotonic() > deadline:
            admission.update(status="timeout", elapsed_seconds=time.monotonic() - started)
            manifest.update(status=timeout_status, timing_qualified=False, error=timeout_error)
            save_manifest()
            return False
        activity = host_activity(strict=True)


def record_idle_snapshot(activity):
    """Copy observations and reject nonstandard JSON constants in evidence."""
    state = idle_snapshot_state(activity)
    try:
        saved = json.loads(json.dumps(activity, allow_nan=False))
    except (TypeError, ValueError) as error:
        state = "unknown"
        saved = {"available": False, "busy_processes": None,
                 "error": f"snapshot cannot be recorded as strict JSON: {error}"}
    return saved, state


def verify_admission_inputs(manifest: dict, output: Path) -> None:
    """Fail closed if any identity frozen before admission has drifted."""
    expected = [(Path(engine["path"]), engine["sha256"])
                for engine in manifest["engines"].values()]
    expected += [(output / "positions.input", manifest["positions_sha256"])]
    expected += [(Path(__file__).parent / name, digest)
                 for name, digest in manifest["measurement_source_sha256"].items()]
    for path, digest in expected:
        if hashlib.sha256(path.read_bytes()).hexdigest() != digest:
            raise ValueError(f"idle admission input changed: {path}")


def signature(row: dict) -> str:
    return json.dumps([row[key] for key in ("bestmove", "score", "pv", "nodes", "depth")])


def work_metrics(row: dict) -> dict:
    result = {key: row[key] for key in ("depth", "nodes", "wall_nps", "elapsed_ms")}
    if "nps" in row:
        result["engine_nps"] = row["nps"]
    lazy = row.get("lazy_smp", {})
    if lazy:
        result["nonselected_nodes"] = lazy["discarded_root_nodes"]
    workers = lazy.get("worker_stats", [])
    if workers:
        result["nonselected_helper_nodes"] = sum(worker["nodes"] for worker in workers if worker["worker"] != 0 and not worker["selected"])
        helpers = [worker for worker in workers if worker["worker"] != 0]
        if helpers:
            result["deepest_helper_depth"] = max(worker["depth"] for worker in helpers)
    main = next((worker for worker in workers if worker["worker"] == 0), None)
    if main:
        result["worker0_depth"] = main["depth"]
    if main and "tt_foreign_hits" in main:
        known = main["tt_local_hits"] + main["tt_foreign_hits"]
        result["worker0_foreign_tt_hits"] = main["tt_foreign_hits"]
        result["worker0_foreign_tt_hit_rate"] = main["tt_foreign_hits"] / known if known else None
    if "node_limit_overshoot" in row:
        result["node_limit_overshoot"] = row["node_limit_overshoot"]
    retirement = row.get("retirement", [])
    if retirement:
        result["helper_completed_iterations"] = sum(item["completed_iterations"] for item in retirement if item["worker"])
        result["retired_helpers"] = sum(item["retired"] for item in retirement if item["worker"])
    if row.get("nmp"):
        keys = ("attempts", "fail_high", "verification_attempts", "verification_successes",
                "verification_overturns", "pv_returns", "tracking_overflow", "prior_reductions")
        for key in keys:
            result[f"nmp_{key}"] = sum(worker[key] for worker in row["nmp"])
    if row.get("aspiration"):
        for column, key in enumerate(ASPIRATION_KEYS):
            result[f"asp_{key}"] = sum(worker["totals"][column] for worker in row["aspiration"])
    waiting = row.get("cooperative_root_wait")
    if waiting:
        for key, value in waiting.items():
            if key != "enabled":
                result[f"coop_{key}"] = value
    if row.get("cooperative_root") is not None:
        result["coop_priority_enabled"] = int(row.get("cooperative_root_policy") is not None)
        result["coop_helper_lmr_enabled"] = int(row.get("cooperative_root_helper_lmr_policy") is not None)
    for field, prefix in (("cooperative_root_prestart", "coop_prestart_"),
                          ("cooperative_root_prestart_cap", "coop_prestart_cap_"),
                          ("cooperative_root_prestart_diag", "coop_prestart_diag_")):
        for key, value in (row.get(field) or {}).items():
            result[prefix + key] = value
    latency = row.get("cooperative_root_latency")
    if latency:
        for key, value in latency.items():
            if key != "enabled":
                result[f"coop_latency_{key}"] = value
    outcomes = row.get("cooperative_root_outcomes")
    if outcomes:
        for key, value in outcomes.items():
            if key != "enabled":
                result[f"coop_outcomes_{key}"] = value
    helper_lmr = row.get("cooperative_root_helper_lmr")
    if helper_lmr:
        for key, value in helper_lmr.items():
            result[f"coop_helper_lmr_{key}"] = value
    return result


def validate_counter_workers(row: dict, count: int) -> None:
    expected = set(range(count))
    for key in ("retirement", "nmp", "aspiration"):
        records = row[key]
        # One-worker diagnostics have no helper progress/retirement channel.
        if key == "retirement" and count == 1 and not records:
            continue
        if {worker["worker"] for worker in records} != expected:
            raise ValueError(f"counter lane requires one {key} record per worker")


def summarize(rows: list[dict], *, fixed_depth: bool, require_identity: bool = True) -> dict:
    blocks = {}
    for row in rows:
        blocks.setdefault((row["repeat"], row["threads"], row["position"]), []).append(row)
    paired = []
    for (repeat, threads, position), block in blocks.items():
        if len(block) != 4 or "".join(row["side"] for row in block) != "ABBA":
            raise ValueError("incomplete or out-of-order ABBA block")
        sides = {side: [row for row in block if row["side"] == side] for side in "AB"}
        item = {"repeat": repeat, "threads": threads, "position": position}
        item["exact_identity"] = len({signature(row) for row in block}) == 1
        item["within_side_nondeterminism"] = {
            side: len({signature(row) for row in side_rows}) != 1 for side, side_rows in sides.items()
        }
        metrics = {side: [work_metrics(row) for row in values] for side, values in sides.items()}
        shared_keys = set.intersection(*(set(row) for group in metrics.values() for row in group))
        item["paired_metrics"] = {}
        item["directional_metrics"] = {"A_first": {}, "B_first": {}}
        for key in sorted(shared_keys):
            values = {side: [row[key] for row in group] for side, group in metrics.items()}
            if any(value is None for group in values.values() for value in group):
                continue
            left, right = (statistics.mean(values[side]) for side in "AB")
            item["paired_metrics"][key] = {"A": left, "B": right, "B_minus_A": right - left,
                                                   "B_over_A": right / left if left else None}
            for direction, a_slot, b_slot in (("A_first", 0, 1), ("B_first", 3, 2)):
                first, second = work_metrics(block[a_slot])[key], work_metrics(block[b_slot])[key]
                item["directional_metrics"][direction][key] = {
                    "B_minus_A": second - first, "B_over_A": second / first if first else None,
                }
        paired.append(item)
    by_threads = {}
    for threads in sorted({row["threads"] for row in rows}):
        subset = [item for item in paired if item["threads"] == threads]
        keys = set.intersection(*(set(item["paired_metrics"]) for item in subset))
        raw_side_medians = {}
        for side in "AB":
            side_metrics = [work_metrics(row) for row in rows if row["threads"] == threads and row["side"] == side]
            side_keys = set.intersection(*(set(record) for record in side_metrics))
            raw_side_medians[side] = {
                key: statistics.median(values) for key in side_keys
                if (values := [record[key] for record in side_metrics if record[key] is not None])
            }
        directional = {
            direction: {
                key: statistics.median(item["directional_metrics"][direction][key]["B_minus_A"] for item in subset)
                for key in keys
            }
            for direction in ("A_first", "B_first")
        }
        by_threads[str(threads)] = {
            "raw_side_medians": raw_side_medians,
            "directional_median_paired_delta": directional,
            "blocks": len(subset),
            "hint_transfer_by_side": {
                side: summarize_lazy_root_hints([row for row in rows if row["threads"] == threads and row["side"] == side])
                for side in "AB"
            },
            "exact_identity_blocks": sum(item["exact_identity"] for item in subset),
            "fixed_depth_identity_required": bool(fixed_depth and threads == 1 and require_identity),
            "identity_pass": all(item["exact_identity"] for item in subset) if fixed_depth and threads == 1 and require_identity else None,
            "within_side_nondeterministic_blocks": sum(any(item["within_side_nondeterminism"].values()) for item in subset),
            "median_paired_delta": {key: statistics.median(item["paired_metrics"][key]["B_minus_A"] for item in subset) for key in keys},
            "median_paired_ratio": {key: statistics.median(values) if (values := [item["paired_metrics"][key]["B_over_A"] for item in subset if item["paired_metrics"][key]["B_over_A"] is not None]) else None for key in keys},
        }
    return {"schema": "sekirei.paired-search-summary.v1", "by_threads": by_threads, "paired_blocks": paired,
            "interpretation": "Multiworker order is nondeterministic. No Elo inference or automatic adoption."}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--positions", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--threads", default="1,2,4,8")
    parser.add_argument("--repeats", type=int, default=2)
    parser.add_argument("--count", type=int, default=20)
    parser.add_argument("--hash", type=int, default=64)
    parser.add_argument("--timeout", type=float, default=120)
    budget = parser.add_mutually_exclusive_group()
    budget.add_argument("--movetime", type=int, default=300)
    budget.add_argument("--depth", type=int)
    budget.add_argument("--nodes", type=positive_u64, help="fixed node budget; unqualified work diagnostic")
    parser.add_argument("--option", action="append", default=[])
    parser.add_argument("--a-option", action="append", default=[])
    parser.add_argument("--b-option", action="append", default=[])
    parser.add_argument("--diagnostics", action="store_true")
    parser.add_argument("--require-transfer", action="store_true")
    parser.add_argument("--policy-diagnostic", action="store_true", help="fixed-depth counter lane with intentionally changed search trees; identity is recorded but not required")
    parser.add_argument("--allow-busy", action="store_true", help="run on a busy/unknown host, recording timing as unqualified")
    parser.add_argument("--idle-wait-seconds", type=float, default=0,
                        help="opt-in bounded idle admission for counter-free movetime runs (0..3600)")
    parser.add_argument("--idle-poll-seconds", type=float, default=10,
                        help="interval between idle snapshots (1..60 seconds)")
    parser.add_argument("--idle-stable-samples", type=int, default=3,
                        help="consecutive idle snapshots required (2..60)")
    parser.add_argument("--idle-min-free-mib", type=int, default=0,
                        help="opt-in output-filesystem free MiB floor; requires idle wait (0 disables)")
    parser.add_argument("--source-ref", help="document actual built source state; never inferred from binary timestamp")
    args = parser.parse_args()
    if option_value(args.option, "MoveOverhead") is None:
        args.option.append("MoveOverhead=0")
    try:
        threads = [int(value) for value in args.threads.split(",")]
        changes = side_changes(args.a_option, args.b_option)
        if not threads or min(threads) < 1 or len(set(threads)) != len(threads):
            raise ValueError("thread counts must be positive and unique")
        if min(args.repeats, args.count, args.movetime, args.hash, args.timeout) < 1 or (args.depth is not None and args.depth < 1):
            raise ValueError("search sizes must be positive")
        if not math.isfinite(args.idle_wait_seconds) or not 0 <= args.idle_wait_seconds <= 3600:
            raise ValueError("idle wait must be finite and between 0 and 3600 seconds")
        if not math.isfinite(args.idle_poll_seconds) or not 1 <= args.idle_poll_seconds <= 60:
            raise ValueError("idle poll must be finite and between 1 and 60 seconds")
        if not 2 <= args.idle_stable_samples <= 60:
            raise ValueError("idle stable samples must be between 2 and 60")
        if args.idle_min_free_mib < 0:
            raise ValueError("idle minimum free MiB must be nonnegative")
        if args.idle_min_free_mib and not args.idle_wait_seconds:
            raise ValueError("idle minimum free MiB requires enabled idle wait")
        if args.idle_wait_seconds:
            if not math.isfinite(args.timeout) or args.timeout <= 0:
                raise ValueError("idle admission requires a finite positive timeout")
            instrumented = any(option_value(args.option + side_options, key) not in (None, "0")
                               for side_options in (args.a_option, args.b_option)
                               for key in ("T_COOP_LATENCY", "T_COOP_PRESTART_DIAG", "T_COOP_DIAG"))
            if args.allow_busy or args.depth is not None or args.nodes is not None or args.diagnostics or instrumented:
                raise ValueError("idle admission requires counter-free movetime without --allow-busy")
            if args.idle_wait_seconds < args.idle_poll_seconds * (args.idle_stable_samples - 1):
                raise ValueError("idle wait must cover poll * (stable samples - 1)")
        if args.policy_diagnostic and (args.depth is None or not args.diagnostics or not changes):
            raise ValueError("policy diagnostic requires fixed depth, counters and one changed factor")
        if args.require_transfer and (not args.diagnostics or min(threads) < 2):
            raise ValueError("transfer lane requires --diagnostics and at least two threads")
        if any(option.partition("=")[0] in {"Threads", "Hash", "EvalFile", "EvalDir", "LazyDiagnostics", "NmpDiagnostics", "AspirationDiagnostics", "UseBook", "SpecTopN", "SearchMode"} for option in args.option + args.a_option + args.b_option):
            raise ValueError("reserved options would violate fixed material-only measurement contract")
        source_data = args.positions.read_bytes()
        positions = []
        for line in source_data.decode("utf-8").splitlines():
            line = line.strip()
            if line and not line.startswith("#"):
                positions.append(line if line.startswith(("startpos", "sfen ")) else "sfen " + line)
        positions = positions[:args.count]
        if not positions:
            raise ValueError("empty position corpus")
    except ValueError as error:
        parser.error(str(error))
    args.output.mkdir(parents=True, exist_ok=False)
    frozen = {}
    for side, source in (("A", args.baseline), ("B", args.candidate)):
        destination = args.output / f"engine-{side}"
        if side == "B" and source.samefile(args.baseline):
            os.link(args.output / "engine-A", destination)
            binary_hash = frozen["A"]["sha256"]
        else:
            binary_hash = freeze(source, destination)
        frozen[side] = {"path": str(destination.resolve()), "sha256": binary_hash, "source_path": str(source.resolve())}
    (args.output / "positions.input").write_bytes(source_data)
    activity = ({"available": False, "busy_processes": None, "error": "idle admission not started"}
                if args.idle_wait_seconds else host_activity())
    latency_requested = {side: option_value(args.option + side_options, "T_COOP_LATENCY") not in (None, "0")
                         for side, side_options in (("A", args.a_option), ("B", args.b_option))}
    priority_requested = {side: option_value(args.option + side_options, "T_COOP_PRIORITY") not in (None, "0")
                          for side, side_options in (("A", args.a_option), ("B", args.b_option))}
    helper_lmr_requested = {side: option_value(args.option + side_options, "T_COOP_HELPER_LMR") not in (None, "0")
                            for side, side_options in (("A", args.a_option), ("B", args.b_option))}
    prestart_diag_requested = {side: option_value(args.option + side_options, "T_COOP_PRESTART_DIAG") not in (None, "0")
                              for side, side_options in (("A", args.a_option), ("B", args.b_option))}
    cap_requested = {side: option_value(args.option + side_options, "T_COOP_PRESTART_CAP") not in (None, "0")
                     for side, side_options in (("A", args.a_option), ("B", args.b_option))}
    cap_explicit = any(option_value(args.option + side_options, "T_COOP_PRESTART_CAP") is not None
                       for side_options in (args.a_option, args.b_option))
    host_idle = activity["available"] and not activity["busy_processes"]
    timing_qualified = args.nodes is None and host_idle and not args.diagnostics and not any(latency_requested.values()) and not any(prestart_diag_requested.values())
    measurement_sources = ["paired_search_diagnostics.py", "nps_threads.py"]
    if args.idle_wait_seconds:
        measurement_sources += ["sekirei_tools/__init__.py", "sekirei_tools/artifacts.py"]
    manifest = {
        "schema": "sekirei.paired-search-manifest.v1", "status": "running",
        "started_at_utc": datetime.now(timezone.utc).isoformat(), "engines": frozen,
        "source_ref": args.source_ref, "source_ref_verified": False,
        "measurement_source_sha256": {name: hashlib.sha256((Path(__file__).parent / name).read_bytes()).hexdigest()
                                      for name in measurement_sources},
        "positions_sha256": hashlib.sha256(source_data).hexdigest(),
        "effective_positions_sha256": effective_positions_sha256(positions),
        "effective_position_count": len(positions), "threads": threads, "repeats": args.repeats,
        "budget": {"depth": args.depth, "movetime_ms": args.movetime if args.depth is None and args.nodes is None else None,
                   **({"node_limit": args.nodes} if args.nodes is not None else {})},
        "evaluator": {"kind": "material-only"}, "hash_mb": args.hash,
        "common_options": args.option, "A_options": args.a_option, "B_options": args.b_option,
        "changed_options": changes, "counter_lane": args.diagnostics,
        "cooperative_latency_instrumentation_requested": latency_requested,
        "cooperative_prestart_instrumentation_requested": prestart_diag_requested,
        **({"cooperative_prestart_cap_requested": cap_requested} if cap_explicit else {}),
        "cooperative_priority_requested": priority_requested,
        "cooperative_helper_lmr_requested": helper_lmr_requested,
        "identity_policy": ("observe_only_fixed_node_work" if args.nodes is not None else
                            "observe_only_policy_change" if args.policy_diagnostic else "require_1T_fixed_depth"),
        **({"measurement_phase": "fixed_node_work_diagnostic"} if args.nodes is not None else {}),
        "host": {"system": platform.system(), "release": platform.release(), "machine": platform.machine(), "cpu_count": os.cpu_count()},
        "schedule": "per-position ABBA; fresh process every slot", "strength_gate_opened": False,
        "host_activity_before": activity, "timing_qualified": timing_qualified,
    }
    configuration = {key: manifest[key] for key in ("threads", "repeats", "budget", "evaluator", "hash_mb", "common_options", "A_options", "B_options", "counter_lane", "identity_policy")}
    if args.idle_wait_seconds:
        request = {"wait_seconds": args.idle_wait_seconds, "poll_seconds": args.idle_poll_seconds,
                   "stable_samples": args.idle_stable_samples}
        if args.idle_min_free_mib:
            request["minimum_free_mib"] = args.idle_min_free_mib
        configuration["idle_admission_request"] = request
        manifest["idle_admission"] = {"schema": "sekirei.idle-search-admission.v1",
                                      "request": request, "status": "waiting", "samples": [],
                                      "elapsed_seconds": 0, "consecutive_idle_samples": 0}
        manifest.update(status="waiting_idle_admission", timing_qualified=False, completed_searches=0)
    manifest["configuration_sha256"] = hashlib.sha256(json.dumps(configuration, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    def save_manifest():
        if args.idle_wait_seconds:
            # Keep freeze()/host snapshot consumers independent of this package.
            import sekirei_tools
            from sekirei_tools import artifacts
            for module, relative in ((sekirei_tools, "sekirei_tools/__init__.py"),
                                     (artifacts, "sekirei_tools/artifacts.py")):
                location = getattr(module, "__file__", None)
                if not location or Path(location).resolve() != (Path(__file__).parent / relative).resolve():
                    raise ValueError("idle admission helper differs from fingerprinted source")
            artifacts.atomic_write_json(args.output / "manifest.json", manifest)
        else:
            (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    save_manifest()
    if args.idle_wait_seconds:
        idle_started = time.monotonic()
        try:
            activity = host_activity(strict=True)
            if not wait_for_idle(manifest, save_manifest, activity, idle_started, request, args.output):
                print(json.dumps({"error": manifest["error"], "idle_admission": manifest["idle_admission"]}))
                return 2
            verify_admission_inputs(manifest, args.output)
            if args.idle_min_free_mib:
                manifest["capacity_before_dispatch"] = capacity_snapshot(args.output, args.idle_min_free_mib)
                if not manifest["capacity_before_dispatch"]["floor_pass"]:
                    manifest.update(status="blocked_capacity_before_dispatch", timing_qualified=False,
                                    error="output filesystem capacity unavailable or below requested floor")
                    save_manifest()
                    return 2
            activity = manifest["host_activity_before"]
            host_idle = True
            manifest.update(status="running", timing_qualified=True)
            save_manifest()
        except BaseException as error:
            manifest["idle_admission"].update(status="incomplete", elapsed_seconds=time.monotonic() - idle_started)
            manifest.update(status="incomplete", error=str(error), completed_searches=0, timing_qualified=False)
            save_manifest()
            raise
    if args.depth is None and args.nodes is None and not args.diagnostics and not host_idle and not args.allow_busy:
        manifest.update(status="blocked_host_busy", error="timing requires an idle host; no processes stopped")
        save_manifest()
        print(json.dumps({"error": manifest["error"], "host_activity": activity}))
        return 2
    rows = []
    try:
        with (args.output / "rows.jsonl").open("x") as output:
            for repeat, count, position, slot, side in schedule(threads, len(positions), args.repeats):
                if args.idle_min_free_mib:
                    capacity = capacity_snapshot(args.output, args.idle_min_free_mib)
                    manifest.setdefault("capacity_before_search", []).append(
                        {"sequence": len(rows), "side": side, "capacity": capacity})
                    manifest["completed_searches"] = len(rows)
                    save_manifest()
                    if not capacity["floor_pass"]:
                        raise ValueError("output filesystem capacity unavailable or below requested floor before search")
                options = ["SearchMode=LazySMP", "SpecTopN=0"] + args.option + (args.a_option if side == "A" else args.b_option)
                require_hint = args.require_transfer and int(option_value(options, "LazyFlags") or 0) & 2048 != 0
                lane = argparse.Namespace(engine=frozen[side]["path"], yaneuraou=False, evalfile=None, fv_scale=24,
                    hash=args.hash, option=options, lazy_diagnostics=args.diagnostics, label=f"{side} slot{slot} ", position_offset=position,
                    require_lazy_diagnostics=args.diagnostics, require_root_hint_diagnostics=require_hint,
                    depth=args.depth, movetime=args.movetime, nodes=args.nodes, repeats=1, timeout=args.timeout, strict_options=True,
                    provenance={"engine_sha256": frozen[side]["sha256"], "effective_positions_sha256": manifest["effective_positions_sha256"],
                                "evaluator": {"kind": "material-only"}, "counter_overhead_included": args.diagnostics or latency_requested[side] or prestart_diag_requested[side],
                                "cooperative_latency_instrumentation_requested": latency_requested[side],
                                "cooperative_prestart_instrumentation_requested": prestart_diag_requested[side],
                                **({"cooperative_prestart_cap_requested": cap_requested[side]} if cap_explicit else {}),
                                "cooperative_priority_requested": priority_requested[side],
                                "cooperative_helper_lmr_requested": helper_lmr_requested[side],
                                **({"measurement_phase": "fixed_node_work_diagnostic", "node_limit": args.nodes,
                                    "timing_qualified": False} if args.nodes is not None else {})})
                row = measure(lane, count, [positions[position]])[0]
                row.update(repeat=repeat, position=position, side=side, abba_slot=slot, sequence=len(rows))
                if require_hint and "first_move_beta_cuts" not in row.get("lazy_smp_root_hint", {}):
                    raise ValueError("transfer lane requires extended root-hint counters")
                if args.diagnostics:
                    validate_counter_workers(row, count)
                rows.append(row)
                output.write(json.dumps(row) + "\n")
                output.flush()
        summary = summarize(rows, fixed_depth=args.depth is not None, require_identity=not args.policy_diagnostic)
        (args.output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
        if args.idle_wait_seconds:
            verify_admission_inputs(manifest, args.output)
        manifest["status"] = "complete"
        if args.idle_wait_seconds:
            manifest["completed_searches"] = len(rows)
        if args.idle_wait_seconds:
            manifest["host_activity_after"], state = record_idle_snapshot(host_activity(strict=True))
            if state != "idle":
                manifest["timing_qualified"] = False
            if args.idle_min_free_mib:
                manifest["capacity_after"] = capacity_snapshot(args.output, args.idle_min_free_mib)
                if not manifest["capacity_after"]["floor_pass"]:
                    manifest["timing_qualified"] = False
        else:
            manifest["host_activity_after"] = host_activity()
            if not manifest["host_activity_after"]["available"] or manifest["host_activity_after"]["busy_processes"]:
                manifest["timing_qualified"] = False
        save_manifest()
        failed_identity = any(item["identity_pass"] is False for item in summary["by_threads"].values())
        print(json.dumps(summary["by_threads"], indent=2))
        return 1 if failed_identity else 0
    except BaseException as error:
        manifest.update(status="incomplete", error=str(error), completed_searches=len(rows), timing_qualified=False)
        save_manifest()
        raise


if __name__ == "__main__":
    raise SystemExit(main())
