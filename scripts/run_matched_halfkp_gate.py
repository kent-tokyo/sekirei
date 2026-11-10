#!/usr/bin/env python3
"""One-shot, sequential matched HalfKP gate; never retries a failed pair.

Run only under run_observed_process.py in a detached session. The technical
smoke corpus is deliberately separate from this formal corpus. A stopped or
interrupted attempt requires an audit before any new launch; this script has
no automatic resume path.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

from scripts import matched_nnue_gate_capacity as capacity
from scripts import nnue_gate_host as host
from scripts.check_nnue_gate_preflight import preflight
from scripts.run_observed_process import append, save


def now() -> str:
    return datetime.now(timezone.utc).isoformat()


def digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(block)
    return value.hexdigest()


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def check_pins(plan: dict) -> None:
    for name, record in plan["pins"].items():
        path = Path(record["path"])
        require(path.is_absolute() and path.is_file()
                and digest(path) == record["sha256"], f"pin changed: {name}")


def validate_plan(plan: dict) -> list[str]:
    schema = plan.get("schema")
    require(schema in {"sekirei.matched-halfkp-formal-gate.v1",
                       "sekirei.matched-halfkp-formal-gate.v2"}, "plan schema")
    require(plan.get("status") == "PREREGISTERED_NO_GAMES", "plan status")
    require(plan.get("candidate") == "hypothesis" and plan.get("baseline") == "control",
            "candidate direction")
    require(plan.get("max_pairs") == 1000 and plan.get("games_per_pair") == 2,
            "formal sample cap")
    require(plan.get("conditions") == {
        "byoyomi_ms": 200, "fv_scale": 24, "hash_mb_each": 64,
        "max_moves": 512, "move_overhead_ms": 0,
        "nnue_output": "absolute", "search_mode": "Speculative",
        "spec_top_n": 0, "threads_each": 1, "use_book": False,
    }, "conditions differ from offline proposal")
    require(plan.get("sprt") == {"alpha": 0.05, "beta": 0.05,
                                  "elo0": 0, "elo1": 40, "variant": "trinomial",
                                  "paired_by_id": True}, "SPRT differs from proposal")
    require(plan.get("resource") == {"start_local_free_mib": 16384,
        "start_archive_free_mib": 8192, "stop_local_floor_mib": 8192,
        "output_cap_bytes": 1024**3, "stable_seconds": 600,
        "max_sample_gap_seconds": 15, "poll_seconds": 5,
        "wait_limit_seconds": 7200, "max_wall_seconds": 86400,
        "pair_timeout_seconds": 1800}, "resource contract")
    require(plan.get("single_attempt") is True and plan.get("automatic_retry") is False,
            "one-shot contract")
    require(plan.get("archive_mount") == "/Volumes/ESD-EPA"
            and plan.get("archive_path") == "/Volumes/ESD-EPA/Projects/sekirei_data",
            "archive path contract")
    pins = plan.get("pins")
    required_pins = {
        "engine", "match", "control", "hypothesis", "roots", "root_manifest",
        "proposal", "seed_reservation", "runner", "observer"}
    if schema.endswith(".v2"):
        required_pins.update({"ledger", "closure"})
    require(isinstance(pins, dict) and set(pins) == required_pins, "pins")
    check_pins(plan)
    manifest = json.loads(Path(pins["root_manifest"]["path"]).read_text())
    require(manifest.get("status") == "complete"
            and manifest.get("requested_count") == 1000
            and len(manifest.get("positions", [])) == 1000
            and len(manifest.get("exclusions", [])) ==
            (50 if schema.endswith(".v1") else plan.get("required_exclusion_count"))
            and manifest.get("openings_sha256") == pins["roots"]["sha256"],
            "unqualified formal roots")
    reservation = json.loads(Path(pins["seed_reservation"]["path"]).read_text())
    require(manifest["configuration"]["seed"] == reservation["formal_seed"]
            and reservation["smoke_seed"] != reservation["formal_seed"], "seed identity")
    roots = Path(pins["roots"]["path"]).read_text().splitlines()
    require(len(roots) == 1000 and all(root.startswith("startpos moves ") for root in roots),
            "formal corpus size or format")
    require(len({row["identity_sha256"] for row in manifest["positions"]}) == 1000,
            "duplicate formal root identity")
    if schema.endswith(".v2"):
        closure = json.loads(Path(pins["closure"]["path"]).read_text())
        ledger = json.loads(Path(pins["ledger"]["path"]).read_text())
        require(plan.get("required_exclusion_count") == closure.get("manifest_count"),
                "closure count changed")
        require(ledger.get("latest_known_closure", {}).get("sha256") ==
                pins["closure"]["sha256"], "ledger closure is stale")
        preflight(plan, ledger, closure)
    return roots


def match_command(plan: dict, position: Path, pair_dir: Path) -> list[str]:
    pins, conditions = plan["pins"], plan["conditions"]
    match, engine = pins["match"]["path"], pins["engine"]["path"]
    command = [match, "--engine1", engine, "--engine2", engine,
               "--positions", str(position), "--games-per-position", "2",
               "--byoyomi", str(conditions["byoyomi_ms"]),
               "--max-moves", str(conditions["max_moves"]),
               "--output", str(pair_dir / "kifu"),
               "--json", str(pair_dir / "result.json")]
    common = ["Threads=1", "Hash=64", "SearchMode=Speculative",
              "SpecTopN=0", "MoveOverhead=0", "NnueOutput=absolute",
              "FV_SCALE=24"]
    for arm, weight in ((1, "hypothesis"), (2, "control")):
        for option in ["EvalFile=" + pins[weight]["path"], *common]:
            command.extend([f"--engine-option{arm}", option])
    return command


def validate_pair(pair_dir: Path, plan: dict) -> list[str]:
    summary = json.loads((pair_dir / "result.json").read_text())
    require(summary.get("status") == "complete" and summary.get("games") == 2
            and summary.get("artifact_files_expected") == 2
            and summary.get("artifact_write_failures") == []
            and summary.get("invalid_games") == [], "pair incomplete or invalid")
    engine = plan["pins"]["engine"]["path"]
    for arm, weight in ((1, "hypothesis"), (2, "control")):
        path = plan["pins"][weight]["path"]
        expected = {"EvalFile": path, "Threads": "1", "Hash": "64",
                    "SearchMode": "Speculative", "SpecTopN": "0",
                    "MoveOverhead": "0", "NnueOutput": "absolute", "FV_SCALE": "24"}
        require(summary.get(f"engine{arm}_command") == engine
                and summary.get(f"engine{arm}_options") == expected
                and summary.get(f"engine{arm}_eval_file_acknowledgement")
                == f"info string NNUE weights loaded from {path}",
                f"engine {arm} identity or options")
    rows = [json.loads(line) for line in (pair_dir / "result.jsonl").read_text().splitlines()]
    require(len(rows) == 2 and all(row.get("id") == "pos0_pair0"
                                   and row.get("result") in
                                   {"candidate_win", "baseline_win", "draw"}
                                   for row in rows), "incomplete colour-reversed pair")
    counts = {name: sum(row["result"] == value for row in rows)
              for name, value in (("engine1_wins", "candidate_win"),
                                  ("engine2_wins", "baseline_win"),
                                  ("draws", "draw"))}
    require(all(summary.get(name) == count for name, count in counts.items()),
            "summary/result mismatch")
    require(len(list((pair_dir / "kifu").glob("game*.txt"))) == 2,
            "missing game record")
    return [row["result"] for row in rows]


def output_bytes(root: Path) -> int:
    return sum(path.stat().st_size for path in root.rglob("*") if path.is_file())


def write_aggregate(outdir: Path, pairs: list[list[str]]) -> Path:
    counts = {"candidate_win": 0, "baseline_win": 0, "draw": 0}
    lines = []
    for index, pair in enumerate(pairs):
        require(len(pair) == 2, "incomplete aggregate pair")
        for result in pair:
            require(result in counts, "invalid aggregate outcome")
            counts[result] += 1
            lines.append(json.dumps({"id": f"pos{index}_pair0", "result": result}) + "\n")
    path = outdir / "combined.json"
    pending = outdir / ".combined.jsonl.pending"
    with pending.open("x") as stream:
        stream.writelines(lines)
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(pending, outdir / "combined.jsonl")
    score = (counts["candidate_win"] + counts["draw"] / 2) / (2 * len(pairs))
    score = min(max(score, 1e-6), 1 - 1e-6)
    approximate_los = 0.5 * (1 + math.erf(
        (score - 0.5) / max(1e-6, 0.5 / math.sqrt(2 * len(pairs)))))
    save(path, {"status": "complete", "games": 2 * len(pairs),
                "engine1_wins": counts["candidate_win"],
                "engine2_wins": counts["baseline_win"], "draws": counts["draw"],
                "elo_diff": 400 * math.log10(score / (1 - score)),
                "los": approximate_los,
                "artifact_write_failures": [], "invalid_games": []})
    return path


def sprt(plan: dict, aggregate: Path) -> tuple[str, str]:
    match = plan["pins"]["match"]["path"]
    command = [match, "gate", str(aggregate), "--sprt", "--elo0", "0",
               "--elo1", "40", "--alpha", "0.05", "--beta", "0.05",
               "--sprt-variant", "trinomial", "--paired-by-id",
               "--require-complete-pairs", "--min-diversity-ratio", "0"]
    result = subprocess.run(command, capture_output=True, text=True, timeout=30)
    labels = [line.split()[0] for line in result.stdout.splitlines()
              if line.split()[:1] and line.split()[0] in
              {"PASS", "FAIL", "INCONCLUSIVE"}]
    expected = {"PASS": 0, "FAIL": 1, "INCONCLUSIVE": 2}
    require(len(labels) == 1 and result.returncode == expected[labels[0]]
            and not result.stderr.strip(), "SPRT tool failed or ambiguous")
    return labels[0], result.stdout


def run(plan_path: Path, outdir: Path) -> None:
    plan = json.loads(plan_path.read_text())
    roots = validate_plan(plan)
    outdir.mkdir(parents=True, exist_ok=True)
    state_path, events = outdir / "state.json", outdir / "events.jsonl"
    require(not state_path.exists(), "attempt already exists; no automatic retry")
    observer_path = outdir / "observer/observer-status.json"
    observer = {}
    for _ in range(50):
        if observer_path.exists():
            observer = json.loads(observer_path.read_text())
            if observer.get("child_pid") == os.getpid():
                break
        time.sleep(0.1)
    require(observer.get("status") == "running"
            and observer.get("child_pid") == os.getpid()
            and observer.get("observer_pid") == os.getppid(),
            "independent observer not attached")
    resource = plan["resource"]
    policy = capacity.CapacityPolicy(
        start_local_free_mib=resource["start_local_free_mib"],
        start_archive_free_mib=resource["start_archive_free_mib"],
        stop_local_floor_mib=resource["stop_local_floor_mib"],
        output_cap_bytes=resource["output_cap_bytes"],
        stable_seconds=resource["stable_seconds"],
        max_sample_gap_seconds=resource["max_sample_gap_seconds"])
    host_policy = host.HostPolicy(physical_cores=10, minimum_free_mib=8192,
                                  stable_samples=3, poll_seconds=5)
    state = {"schema": "sekirei.matched-halfkp-gate-run.v1", "status": "WAITING_RESOURCES",
             "started_at": now(), "plan_sha256": digest(plan_path), "completed_pairs": 0,
             "formal_games_retained": 0, "qualification_games": 0,
             "automatic_retry": False, "pid": os.getpid(),
             "observer_pid": os.getppid(), "deadline_utc": None}
    save(state_path, state)
    append(events, {"at": now(), "event": "one_shot_started"})
    begin = time.monotonic()
    window = None
    host_count = 0
    pairs = []
    deadline = None
    try:
        while len(pairs) < len(roots):
            sample_at = time.monotonic()
            disk = capacity.observe(outdir, Path(plan["archive_path"]),
                                    Path(plan["archive_mount"]))
            cpu = host.capture(outdir, host_policy)
            append(outdir / "resource.jsonl", {"at": now(), "elapsed_seconds":
                   round(sample_at - begin, 1), "disk": disk, "host": cpu})
            if deadline is None:
                window = capacity.advance_window(window, sample_at, disk, policy)
                host_count = host_policy.next_stable_count(host_count, cpu)
                admitted = (capacity.start_admitted(window, sample_at, disk, policy)
                            and host_count >= host_policy.stable_samples)
                if not admitted:
                    if sample_at - begin >= resource["wait_limit_seconds"]:
                        state.update(status="RESOURCE_WAIT_EXPIRED", ended_at=now())
                        break
                    time.sleep(resource["poll_seconds"])
                    continue
                deadline = time.monotonic() + resource["max_wall_seconds"]
                state.update(status="RUNNING", admitted_at=now(),
                             deadline_utc=datetime.fromtimestamp(
                                 time.time() + resource["max_wall_seconds"],
                                 timezone.utc).isoformat())
                save(state_path, state)
                append(events, {"at": now(), "event": "capacity_admitted",
                                "deadline_utc": state["deadline_utc"]})
            if time.monotonic() >= deadline:
                state.update(status="DEADLINE_INCONCLUSIVE", ended_at=now())
                break
            if capacity.after_pair_action(disk, output_bytes(outdir), policy) != "continue_if_other_gates_pass":
                state.update(status="RESOURCE_STOP", ended_at=now())
                break
            host_count = host_policy.next_stable_count(host_count, cpu)
            if host_count < host_policy.stable_samples:
                time.sleep(resource["poll_seconds"])
                continue
            check_pins(plan)
            index = len(pairs)
            pair_dir = outdir / f"pair-{index:04d}"
            pair_dir.mkdir()
            position = pair_dir / "position.usi"
            position.write_text(roots[index] + "\n")
            command = match_command(plan, position, pair_dir)
            save(pair_dir / "command.json", {"argv": command, "index": index,
                                              "at": now(), "automatic_retry": False})
            state.update(active_pair=index, active_pid=None)
            save(state_path, state)
            append(events, {"at": now(), "event": "pair_starting", "index": index})
            with (pair_dir / "stdout.log").open("x") as stdout, \
                 (pair_dir / "stderr.log").open("x") as stderr:
                child = subprocess.Popen(command, stdout=stdout, stderr=stderr,
                                         stdin=subprocess.DEVNULL)
                state["active_pid"] = child.pid
                save(state_path, state)
                try:
                    returncode = child.wait(timeout=resource["pair_timeout_seconds"])
                except subprocess.TimeoutExpired:
                    # Our observer owns this runner's process group. Terminate
                    # the whole group so no engine process can be orphaned.
                    append(events, {"at": now(), "event": "pair_timeout", "index": index})
                    os.killpg(os.getpgrp(), signal.SIGTERM)
                    raise RuntimeError("pair timeout")
            require(returncode == 0, f"pair {index} exited {returncode}")
            results = validate_pair(pair_dir, plan)
            pairs.append(results)
            aggregate = write_aggregate(outdir, pairs)
            verdict, report = sprt(plan, aggregate)
            save(pair_dir / "gate-check.json", {"verdict": verdict,
                 "report": report, "combined_sha256": digest(aggregate), "at": now()})
            state.update(completed_pairs=len(pairs), formal_games_retained=2 * len(pairs),
                         active_pair=None, active_pid=None, sprt=verdict,
                         status="RUNNING")
            save(state_path, state)
            append(events, {"at": now(), "event": "pair_retained",
                            "index": index, "results": results, "sprt": verdict})
            if verdict in {"PASS", "FAIL"}:
                state.update(status="H1" if verdict == "PASS" else "H0",
                             qualification_games=2 * len(pairs), ended_at=now())
                break
            disk = capacity.observe(outdir, Path(plan["archive_path"]),
                                    Path(plan["archive_mount"]))
            if capacity.after_pair_action(disk, output_bytes(outdir), policy) != "continue_if_other_gates_pass":
                state.update(status="RESOURCE_STOP", ended_at=now())
                break
        else:
            state.update(status="INCONCLUSIVE", qualification_games=2 * len(pairs),
                         ended_at=now())
        check_pins(plan)
        save(state_path, state)
        append(events, {"at": now(), "event": "terminal", "status": state["status"],
                        "completed_pairs": len(pairs)})
    except BaseException as error:
        state.update(status="FAILED_NEEDS_AUDIT", ended_at=now(),
                     error={"type": type(error).__name__, "message": str(error)},
                     completed_pairs=len(pairs))
        save(state_path, state)
        append(events, {"at": now(), "event": "failure", "error": state["error"]})
        raise


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plan", type=Path, required=True)
    parser.add_argument("--outdir", type=Path, required=True)
    args = parser.parse_args()
    run(args.plan, args.outdir)
    return 0


if __name__ == "__main__":
    sys.exit(main())
