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
import os
import platform
import shutil
import subprocess
import statistics
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


def host_activity() -> dict:
    """Record a read-only process snapshot; never stop another task's process."""
    try:
        result = subprocess.run(["ps", "-Ao", "pid,pcpu,comm"], capture_output=True,
                                text=True, check=True, timeout=10)
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
    activity = host_activity()
    latency_requested = {side: option_value(args.option + side_options, "T_COOP_LATENCY") not in (None, "0")
                         for side, side_options in (("A", args.a_option), ("B", args.b_option))}
    priority_requested = {side: option_value(args.option + side_options, "T_COOP_PRIORITY") not in (None, "0")
                          for side, side_options in (("A", args.a_option), ("B", args.b_option))}
    helper_lmr_requested = {side: option_value(args.option + side_options, "T_COOP_HELPER_LMR") not in (None, "0")
                            for side, side_options in (("A", args.a_option), ("B", args.b_option))}
    prestart_diag_requested = {side: option_value(args.option + side_options, "T_COOP_PRESTART_DIAG") not in (None, "0")
                              for side, side_options in (("A", args.a_option), ("B", args.b_option))}
    host_idle = activity["available"] and not activity["busy_processes"]
    timing_qualified = args.nodes is None and host_idle and not args.diagnostics and not any(latency_requested.values()) and not any(prestart_diag_requested.values())
    manifest = {
        "schema": "sekirei.paired-search-manifest.v1", "status": "running",
        "started_at_utc": datetime.now(timezone.utc).isoformat(), "engines": frozen,
        "source_ref": args.source_ref, "source_ref_verified": False,
        "measurement_source_sha256": {name: hashlib.sha256(Path(__file__).with_name(name).read_bytes()).hexdigest()
                                      for name in ("paired_search_diagnostics.py", "nps_threads.py")},
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
    manifest["configuration_sha256"] = hashlib.sha256(json.dumps(configuration, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    def save_manifest():
        (args.output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    save_manifest()
    if args.depth is None and args.nodes is None and not args.diagnostics and not host_idle and not args.allow_busy:
        manifest.update(status="blocked_host_busy", error="timing requires an idle host; no processes stopped")
        save_manifest()
        print(json.dumps({"error": manifest["error"], "host_activity": activity}))
        return 2
    rows = []
    try:
        with (args.output / "rows.jsonl").open("x") as output:
            for repeat, count, position, slot, side in schedule(threads, len(positions), args.repeats):
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
        manifest["status"] = "complete"
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
