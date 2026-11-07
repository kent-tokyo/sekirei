#!/usr/bin/env python3
"""Produce private two-depth observations using Sekirei's native search diagnostic.

Request schema: sekirei.owned-teacher-producer-request.v1. Supply the preflight's
provenance, scales, thresholds and hash-bound train/validation TSVs, profiles
containing ONLY requested_depth, binary={path,sha256}, and weights={path,sha256}
only for owned-halfkp. No observations can be imported. teacher.search requires:
  threads=1, spec_top_n=0, use_book=false, hash_mb=64,
  tt_policy=clear-per-position, backend=search-diagnostic-v1,
  teacher_search=true, root_mate_safety=false,
  qsearch={source_commit: <teacher commit>, configuration: compiled-defaults}.
The CLI has fixed Hash=64; FV_SCALE is passed explicitly and checked in output.

limits: {per_position_seconds: 10, cleanup_seconds: 1, max_wall_seconds: 120,
         minimum_free_bytes: 1073741824, max_positions: 16}.
attempt: {reason: <nonempty>, previous_receipt: null | {path,sha256}}.
Every retry needs a NEW output directory and a reason; existing directories are
never resumed. Prior receipts link evidence, never observations. Only declared
private Sekirei-owned data/material or owned-halfkp teachers are accepted.
Source-to-binary mapping and corpus ownership remain caller attestations.

Each position/depth runs a fresh pinned search_diagnostic process with only
--max-depth N --teacher-search --fv-scale N --sfen SFEN
[--weights FILE --nnue-output absolute].
No node/time search cap or warmup is injected. The parent's absolute wall timeout
terminates/reaps the process and NEVER salvages a partial score. --teacher-search
omits production root-mate safety by design; it is a teacher label search, not
production move selection. Source: crates/sekirei-core/src/bin/search_diagnostic.rs.

Its TSV must explicitly provide depth, bound, completed_bound, aborted,
abort_reason, completed_iteration_valid, teacher_search, score_cp, bestmove,
pv_usi, pv_legal, pv_replay_preserves_input and history fields. Missing completion
proof is incomplete. No missing bound is inferred exact. Native mate sentinels
(abs(score)>=899000, MATE_SCORE=900000) are converted to mate observations and
excluded from CP comparisons. Generic USI is intentionally unsupported: its
current normalization drops SearchInfo completion fields. A future USI extension
would need those same explicit fields bound to the final score/PV/bestmove.

All failures are retained. On the first engine failure, timeout, capacity loss or
interruption, stop launching and write remaining rows as incomplete/not_started.
A completion reserve helps record capacity stops; SIGKILL/power loss/ENOSPC can
still leave only started.json plus partial JSONL. Such attempts are unknown and
never resumed. A per-teacher lock in the evidence parent prevents concurrent
launches; an unreaped child or hard interruption keeps it for a process audit.
No preflight input manifest follows input mutation/write failure.
SFEN-only probes have no repetition history. No build, training, games or model
adoption occurs here. Tests use only owned synthetic corpora and mock processes.
"""

from __future__ import annotations

import argparse
from collections import Counter
from contextlib import closing, contextmanager
import copy
import hashlib
import json
import math
import os
from pathlib import Path
import select
import shutil
import signal
import sqlite3
import subprocess
import time
from datetime import datetime, timezone

from compare_halfkp_parameters import inspect_file, write_new_report
from diagnose_owned_halfkp import bind_ref, index_corpus
from preflight_owned_teacher_stability import PROFILES, SCHEMA as PREFLIGHT_SCHEMA
from preflight_owned_teacher_stability import normalized_observation, run_identity, validate_manifest
from sekirei_tools.artifacts import file_sha256

SCHEMA = "sekirei.owned-teacher-producer-request.v1"
MAX_WIRE = 65536
MAX_LINE = 16384
ENVIRONMENT = {"LANG": "C", "LC_ALL": "C", "TZ": "UTC"}


class ProtocolError(ValueError):
    pass


class CapacityError(RuntimeError):
    pass


def native_command(binary, weights, sfen, depth, fv_scale):
    command = [str(binary), "--max-depth", str(depth), "--teacher-search", "--fv-scale", str(fv_scale), "--sfen", sfen]
    if weights is not None:
        command.extend(("--weights", str(weights), "--nnue-output", "absolute"))
    return command


def classify(payload, sfen, depth, fv_scale):
    lines = [line for line in payload.splitlines() if line.startswith("bestmove=")]
    if not lines:
        return {"status": "incomplete", "observation": None, "reason": "native_completion_record_missing"}
    if len(lines) != 1:
        raise ProtocolError("exactly one native final result required")
    fields = {}
    for token in lines[0].split("\t"):
        key, separator, value = token.partition("=")
        if not separator or key in fields:
            raise ProtocolError("malformed or duplicate native result field")
        fields[key] = value
    required = {"depth", "bound", "completed_bound", "aborted", "abort_reason", "score_cp", "bestmove",
                "completed_iteration_valid", "teacher_search", "pv_usi", "pv_legal",
                "pv_replay_preserves_input", "history_replayed", "history_moves", "history_matches_expected", "fv_scale"}
    if not required <= fields.keys():
        return {"status": "incomplete", "observation": None, "reason": "native_bound_completion_proof_missing"}
    for key in ("aborted", "completed_iteration_valid", "teacher_search", "pv_legal",
                "pv_replay_preserves_input", "history_replayed", "history_matches_expected"):
        if fields[key] not in {"true", "false"}:
            raise ProtocolError("native proof bool must be explicit true/false")
    if (fields["teacher_search"] != "true" or fields["history_replayed"] != "false"
            or fields["history_moves"] != "0" or fields["history_matches_expected"] != "true"
            or fields["fv_scale"] != str(fv_scale)):
        raise ProtocolError("native teacher/history settings differ from requested contract")
    if fields["pv_legal"] != "true" or fields["pv_replay_preserves_input"] != "true":
        raise ProtocolError("native PV legality/input-preservation verification failed")
    reached, score = int(fields["depth"]), int(fields["score_cp"])
    if not 0 <= reached <= depth or not -900000 <= score <= 900000:
        raise ProtocolError("native depth or score out of range")
    if fields["bestmove"] == "resign" and fields["aborted"] == "false" and fields["abort_reason"] == "none":
        return {"status": "terminal", "observation": None, "reason": "no_native_bestmove", "native": fields}
    if (fields["aborted"] != "false" or fields["abort_reason"] != "none"
            or fields["completed_iteration_valid"] != "true" or reached != depth
            or fields["bound"] not in {"exact", "lower", "upper"}
            or fields["completed_bound"] != fields["bound"]):
        return {"status": "incomplete", "observation": None, "reason": "native_search_not_complete", "native": fields}
    if fields["pv_usi"].split(",")[0] != fields["bestmove"]:
        raise ProtocolError("native PV does not start with bestmove")
    kind = "mate" if abs(score) >= 899000 else "cp"
    value = (900000 - abs(score)) * (1 if score > 0 else -1) if kind == "mate" else score
    if kind == "mate" and value == 0:
        return {"status": "terminal", "observation": None, "reason": "native_mate_at_root", "native": fields}
    observation = {"depth": reached, "score": {"kind": kind, "value": value},
                   "score_perspective": "side_to_move", "score_bound": fields["bound"],
                   "bestmove": fields["bestmove"], "was_timeout_salvaged": False}
    normalized_observation({"sfen": sfen, "status": "ok", "observation": observation}, depth)
    return {"status": "ok", "observation": observation, "reason": "native_proof_verified", "native": fields}


def probe(binary, weights, request, sfen, depth, deadline, cwd):
    """Bound stdout/stderr together; parent-only timeout, no partial-score salvage."""
    proc, wire = None, bytearray()
    row = {"status": "engine_error", "observation": None}
    started = time.monotonic()
    command = native_command(binary, weights, sfen, depth, request["scales"]["fv_scale"])
    try:
        proc = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                stderr=subprocess.STDOUT, cwd=str(cwd), env=ENVIRONMENT,
                                start_new_session=True, bufsize=0)
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("position deadline expired")
            ready, _, _ = select.select([proc.stdout], [], [], remaining)
            if not ready:
                raise TimeoutError("position response deadline expired")
            chunk = os.read(proc.stdout.fileno(), min(4096, MAX_WIRE + 1 - len(wire)))
            if not chunk:
                break
            wire.extend(chunk)
            if len(wire) > MAX_WIRE:
                raise ProtocolError("native transcript byte cap exceeded")
        if any(len(line) > MAX_LINE for line in wire.splitlines()):
            raise ProtocolError("native transcript line cap exceeded")
        code = proc.wait(timeout=max(0.001, deadline - time.monotonic()))
        if code != 0:
            raise ProtocolError("native diagnostic exited unsuccessfully")
        row = classify(wire.decode("utf-8", errors="strict"), sfen, depth, request["scales"]["fv_scale"])
    except (KeyboardInterrupt, InterruptedError) as error:
        row = {"status": "incomplete", "observation": None, "reason": type(error).__name__, "interrupted": True}
    except (TimeoutError, subprocess.TimeoutExpired) as error:
        row = {"status": "timeout", "observation": None, "reason": str(error)}
    except (OSError, ValueError, RuntimeError) as error:
        row = {"status": "engine_error", "observation": None, "reason": str(error)}
    finally:
        if proc is not None:
            killed = proc.poll() is None
            try:
                if killed:
                    try:
                        os.killpg(proc.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    except OSError as error:
                        row.update(status="engine_error", observation=None, reason="native kill failed: " + str(error))
                try:
                    code = proc.wait(timeout=request["limits"]["cleanup_seconds"])
                    row["termination"] = {"pid": proc.pid, "exit_code": code, "killed": killed, "reaped": True}
                except subprocess.TimeoutExpired:
                    row.update(status="engine_error", observation=None, reason="native process not confirmed reaped")
                    row["termination"] = {"pid": proc.pid, "exit_code": None, "killed": killed, "reaped": False}
            finally:
                proc.stdout.close()
    row.update(command=command, transcript=wire.decode("utf-8", errors="replace"),
               wall_seconds=time.monotonic() - started)
    return row


def check_capacity(path, floor, remaining):
    free = shutil.disk_usage(path).free
    if type(free) is not int or free < floor + remaining:
        raise CapacityError("output filesystem below floor plus remaining output budget")
    return {"free_bytes": free, "floor_bytes": floor, "remaining_budget_bytes": remaining}


def validate_request(request):
    if not isinstance(request, dict) or request.get("schema") != SCHEMA:
        raise ValueError("unsupported producer request schema")
    template = copy.deepcopy(request)
    template["schema"] = PREFLIGHT_SCHEMA
    if not isinstance(template.get("profiles"), dict) or set(template["profiles"]) != set(PROFILES):
        raise ValueError("exactly shallow and deep profiles required")
    for profile in template["profiles"].values():
        if not isinstance(profile, dict) or set(profile) != {"requested_depth"}:
            raise ValueError("producer profiles cannot import observations or extra settings")
        profile["observations"] = None
    _, target, mix = validate_manifest(template)
    teacher = request["provenance"]["teacher"]
    search = teacher["search"]
    if set(search) != {"threads", "spec_top_n", "use_book", "hash_mb", "tt_policy", "qsearch",
                       "backend", "teacher_search", "root_mate_safety"}:
        raise ValueError("producer supports only its fixed search profile")
    if search["qsearch"] != {"source_commit": teacher["source_commit"], "configuration": "compiled-defaults"}:
        raise ValueError("qsearch must be explicitly pinned to this source's compiled defaults")
    if (search["hash_mb"] != 64 or search["backend"] != "search-diagnostic-v1"
            or search["teacher_search"] is not True or search["root_mate_safety"] is not False):
        raise ValueError("native CLI requires Hash=64 and explicit teacher-search without root safety")
    limits = request.get("limits")
    if not isinstance(limits, dict) or set(limits) != {"per_position_seconds", "cleanup_seconds", "max_wall_seconds", "minimum_free_bytes", "max_positions"}:
        raise ValueError("all producer resource limits must be explicit")
    for key, maximum in (("per_position_seconds", 60), ("cleanup_seconds", 5), ("max_wall_seconds", 3600)):
        value = limits[key]
        if type(value) not in (int, float) or not math.isfinite(value) or not 0 < value <= maximum:
            raise ValueError(f"invalid finite bounded {key}")
    if type(limits["minimum_free_bytes"]) is not int or limits["minimum_free_bytes"] < 1024**3:
        raise ValueError("minimum_free_bytes must preserve at least 1 GiB")
    if type(limits["max_positions"]) is not int or not 1 <= limits["max_positions"] <= 128:
        raise ValueError("max_positions must be in 1..128 for this small producer")
    attempt = request.get("attempt")
    if (not isinstance(attempt, dict) or set(attempt) != {"reason", "previous_receipt"}
            or not isinstance(attempt["reason"], str) or not attempt["reason"].strip()):
        raise ValueError("attempt reason and explicit previous_receipt required")
    return template, target, mix


def durable_row(stream, row):
    stream.write(json.dumps(row, ensure_ascii=False, sort_keys=True, allow_nan=False) + "\n")
    stream.flush()
    os.fsync(stream.fileno())


def stat_identity(path):
    stat = path.stat()
    return (stat.st_dev, stat.st_ino, stat.st_size, stat.st_mtime_ns, stat.st_ctime_ns)


@contextmanager
def interruption_handler():
    def interrupt(_signum, _frame):
        raise InterruptedError("SIGTERM received")
    old = signal.signal(signal.SIGTERM, interrupt)
    try:
        yield
    finally:
        signal.signal(signal.SIGTERM, old)


def produce(request_path, output_dir, *, runner=probe):
    started = time.monotonic()
    request_path = Path(request_path).resolve(strict=True)
    payload = request_path.read_bytes()
    request = json.loads(payload)
    template, target, mix = validate_request(request)
    output = Path(output_dir).absolute()
    output.mkdir(mode=0o700, parents=False, exist_ok=False)
    os.chmod(output, 0o700)
    bindings = {str(request_path): hashlib.sha256(payload).hexdigest()}
    receipt = {"schema": "sekirei.owned-teacher-producer-receipt.v1", "status": "started",
               "started_at_utc": datetime.now(timezone.utc).isoformat(), "request_sha256": bindings[str(request_path)],
               "attempt": request["attempt"], "tool_sha256": file_sha256(Path(__file__)),
               "environment": ENVIRONMENT, "output_directory": str(output), "strength_claim": False,
               "training_permitted": False, "counts": {}, "inputs": bindings}
    write_new_report(output / "started.json", receipt, [request_path])
    reserve = output / "completion-reserve.bin"
    streams = {}
    queue = []
    written = 0
    mutable_inputs = False
    writing = False
    lock_owned = False
    unreaped = False
    lock = output.parent / (".teacher-" + request["provenance"]["teacher"]["binary_sha256"] + ".lock")
    try:
        root = request_path.parent
        paths = {arm: bind_ref(root, request.get(arm), bindings) for arm in ("train", "validation")}
        if sum(path.stat().st_size for path in paths.values()) > 8 * 1024**2:
            raise ValueError("small-producer corpus byte cap is 8 MiB")
        binary = bind_ref(root, request.get("binary"), bindings)
        teacher = request["provenance"]["teacher"]
        if bindings[str(binary)] != teacher["binary_sha256"]:
            raise ValueError("teacher binary hash differs from provenance")
        weights = None
        if teacher["evaluator"] == "owned-halfkp":
            weights = bind_ref(root, request.get("weights"), bindings)
            if bindings[str(weights)] != teacher["weights_sha256"]:
                raise ValueError("teacher weights hash differs from provenance")
            inspect_file(weights)
        elif request.get("weights") is not None:
            raise ValueError("material teacher cannot supply weights")
        if request["attempt"]["previous_receipt"] is not None:
            prior = bind_ref(root, request["attempt"]["previous_receipt"], bindings)
            previous = json.loads(prior.read_text())
            if (previous.get("schema") != receipt["schema"]
                    or previous.get("status") not in {"complete_observations", "failed", "interrupted"}):
                raise ValueError("previous receipt must be a terminal producer receipt")
        snapshot = {path: stat_identity(Path(path)) for path in bindings}
        with closing(sqlite3.connect(str(output / "corpus-index.sqlite"))) as connection:
            receipt["corpus"] = index_corpus(connection, paths, target, mix)
            positions = [raw.split("\t")[0] for (raw,) in connection.execute("SELECT raw FROM positions ORDER BY id")]
        (output / "corpus-index.sqlite").unlink()
        if len(positions) > request["limits"]["max_positions"]:
            raise ValueError("unique positions exceed registered maximum")
        for sfen in positions:
            for profile in PROFILES:
                depth = template["profiles"][profile]["requested_depth"]
                queue.append({"profile": profile, "sfen": sfen, "requested_depth": depth,
                              "run_identity_sha256": run_identity(teacher, request["scales"]["fv_scale"], depth)})
        limits = request["limits"]
        record_budget = MAX_WIRE * 6 + 8192
        output_budget = len(queue) * record_budget + 65536
        receipt["admission"] = check_capacity(output, limits["minimum_free_bytes"], output_budget)
        needed_time = len(queue) * (limits["per_position_seconds"] + 2 * limits["cleanup_seconds"]) + 2
        deadline = started + limits["max_wall_seconds"]
        if deadline - time.monotonic() < needed_time:
            raise CapacityError("wall budget cannot admit all planned observations plus cleanup")
        # One teacher process lane per private evidence parent. A SIGKILL can
        # leave this lock; never infer that a stale-looking lock is safe to reuse.
        with lock.open("x", encoding="utf-8") as stream:
            lock_owned = True
            json.dump({"pid": os.getpid(), "output_directory": str(output),
                       "request_sha256": receipt["request_sha256"]}, stream)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        with reserve.open("xb") as stream:
            stream.write(b"\0" * (len(queue) * 2048 + 65536))
            stream.flush()
            os.fsync(stream.fileno())
        for profile in PROFILES:
            streams[profile] = (output / (profile + ".jsonl")).open("x", encoding="utf-8")
        receipt["status"] = "complete_observations"
        receipt["counts"] = {profile: Counter() for profile in PROFILES}
        for item in queue:
            check_capacity(output, limits["minimum_free_bytes"], (len(queue)-written) * record_budget)
            if deadline - time.monotonic() < limits["per_position_seconds"] + 2 * limits["cleanup_seconds"] + 2:
                raise CapacityError("attempt deadline reached before next observation")
            if any(stat_identity(Path(path)) != value for path, value in snapshot.items()):
                mutable_inputs = True
                raise ValueError("input changed before observation")
            row = runner(binary, weights, request, item["sfen"], item["requested_depth"],
                         min(deadline, time.monotonic() + limits["per_position_seconds"]), output)
            record = item | row
            unreaped = row.get("termination", {}).get("reaped") is False
            writing = True
            durable_row(streams[item["profile"]], record)
            writing = False
            receipt["counts"][item["profile"]][row["status"]] += 1
            written += 1
            if row.get("interrupted"):
                raise InterruptedError("teacher process interrupted; remaining observations not started")
            if row["status"] in {"timeout", "engine_error"}:
                raise RuntimeError("teacher failed; remaining observations not started")
    except (KeyboardInterrupt, InterruptedError) as error:
        receipt.update(status="interrupted", reason=type(error).__name__ + ": " + str(error))
    except (OSError, ValueError, RuntimeError, sqlite3.Error) as error:
        receipt.update(status="failed", reason=type(error).__name__ + ": " + str(error))
    finally:
        reserve.unlink(missing_ok=True)
        if queue and not writing:
            for profile in PROFILES:
                if profile not in streams:
                    streams[profile] = (output / (profile + ".jsonl")).open("x", encoding="utf-8")
                receipt["counts"].setdefault(profile, Counter())
            for item in queue[written:]:
                row = item | {"status": "incomplete", "observation": None,
                              "reason": "not_started_after_" + receipt["status"]}
                durable_row(streams[item["profile"]], row)
                receipt["counts"][item["profile"]]["incomplete"] += 1
        for stream in streams.values():
            stream.close()
        if lock_owned and not unreaped:
            lock.unlink()
        if lock_owned and unreaped:
            receipt["retained_process_lock"] = str(lock)
    for path, expected in bindings.items():
        if not Path(path).is_file() or file_sha256(Path(path)) != expected:
            mutable_inputs = True
    if mutable_inputs:
        receipt.update(status="failed", reason="bound input mutated; observations cannot be combined or accepted")
    if queue and not mutable_inputs and not writing:
        template = {key: template[key] for key in ("schema", "provenance", "scales", "thresholds", "train", "validation", "profiles")}
        for arm, path in paths.items():
            template[arm] = {"path": str(path), "sha256": bindings[str(path)]}
        for profile in PROFILES:
            path = output / (profile + ".jsonl")
            template["profiles"][profile]["observations"] = {"path": path.name, "sha256": file_sha256(path)}
        template["producer_attempt"] = {"request_sha256": receipt["request_sha256"], "output_directory": str(output), "status": receipt["status"]}
        write_new_report(output / "preflight-inputs.json", template, bindings)
    receipt["elapsed_seconds"] = time.monotonic() - started
    receipt["outputs"] = {path.name: file_sha256(path) for path in output.glob("*.jsonl")}
    write_new_report(output / "receipt.json", receipt, bindings)
    return receipt


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("request", type=Path)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args(argv)
    previous_umask = os.umask(0o077)
    try:
        with interruption_handler():
            receipt = produce(args.request, args.output_dir)
    except (OSError, ValueError, RuntimeError, sqlite3.Error) as error:
        parser.exit(2, f"producer failed: {error}\n")
    finally:
        os.umask(previous_umask)
    print(json.dumps(receipt, ensure_ascii=False, sort_keys=True))
    return 0 if receipt["status"] == "complete_observations" else 2


if __name__ == "__main__":
    raise SystemExit(main())
