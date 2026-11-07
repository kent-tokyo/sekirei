#!/usr/bin/env python3
"""Compare Lazy SMP and bounded speculation in fresh-process, timed ABBA blocks.

One frozen Sekirei executable runs both modes. Only 4T/8T wall-time budgets
are supported: Lazy SMP's per-worker node limit cannot be equated to the
speculative invocation's aggregate node limit. This is a search diagnostic,
never a strength gate. --prepare-only freezes a reviewable plan without
starting engines; execution requires the recorded resource root and capacity.
--lane counters uses different observers for the two modes, so its durations and
node counts are not comparable performance measurements. Use a separate output
directory and --lane timing (the default) for counter-free comparisons.
Version 2 requires --legal-moves and successful, hash-bound root-history/PV
replay before a completed run can qualify its timing. This is a same-rules
contract check, not an independent shogi oracle or a playing-strength gate.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import platform
import queue
import re
import shutil
import statistics
import subprocess
import threading
import time
from datetime import datetime, timezone
from pathlib import Path

from nps_threads import parse_lazy_smp, parse_search_result
from paired_search_diagnostics import freeze, host_activity, idle_snapshot_state, schedule


SCHEMA = "sekirei.speculative-paired-diagnostic.v2"
MOVE = re.compile(r"(?:[1-9][a-i][1-9][a-i]\+?|[PLNSGBR]\*[1-9][a-i])\Z")
SOURCES = ("paired_speculative_diagnostics.py", "paired_search_diagnostics.py", "nps_threads.py",
           "validate_paired_pv.py", "generate_search_diagnostic.py")
SPEC_FIELDS = tuple("version configured_workers background_cap candidates admitted started completed aborted unwound unstarted cap_rejected peak_in_flight outstanding_after_join background_nodes elapsed_ns scope_join_wait_ns deadline_overrun_ns cancellation_latency_samples cancellation_latency_ns_sum cancellation_latency_ns_max aborted_without_observed_cancel cancellation_scope".split())
TT_USE_COUNTERS = tuple("score_hits_from_speculation move_hits_from_speculation unattributed_score_hits main_cutoffs_from_speculation quiescence_cutoffs_from_speculation slack_cutoffs_from_speculation probcut_cutoffs_from_speculation nonroot_move_first_started nonroot_move_first_completed nonroot_move_first_beta_cuts".split())
TT_USE_FIELDS = ("version", "available") + TT_USE_COUNTERS + ("source_scope", "consumer_scope", "ordering_scope", "benefit_scope")
LITERAL_FIELDS = {"cancellation_scope": "explicit_request_to_capture_release",
                  "source_scope": "current_invocation_speculation", "consumer_scope": "foreground_tree",
                  "ordering_scope": "nonroot_first_child", "benefit_scope": "observed_kernel_use"}
LAZY_TT_FIELDS = tuple("probes hits attempted committed same_hash equal_depth shallower_rejected collision_overwrites".split())
LAZY_OVERLAP_FIELDS = tuple("sample_rate visits unique within_worker_revisits cross_worker_visits shared_fingerprints dropped tt_sampled_stores tt_sampled_hits tt_local_hits tt_foreign_hits tt_unknown_hits tt_dropped_stores".split())


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def save_json(path: Path, value: dict) -> None:
    temporary = path.with_suffix(path.suffix + ".tmp")
    with temporary.open("w", encoding="utf-8") as stream:
        json.dump(value, stream, indent=2, sort_keys=True, allow_nan=False)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())
    temporary.replace(path)


def positions_from_text(text: str) -> list[str]:
    result = []
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        fields = line.split()
        if fields[0] not in {"startpos", "sfen"}:
            fields.insert(0, "sfen")
        base = 1 if fields[0] == "startpos" else 5
        if len(fields) < base:
            raise ValueError("position requires startpos or four SFEN fields")
        if base == 5:
            board, side, hand, ply = fields[1:5]
            if (len(board.split("/")) != 9 or not re.fullmatch(r"[1-9/PLNSGBRKplnsgbrk+]+", board)
                    or side not in {"b", "w"} or not re.fullmatch(r"-|[1-9PLNSGBRplnsgbr]+", hand)
                    or not ply.isascii() or not ply.isdecimal() or int(ply) < 1):
                raise ValueError("invalid SFEN syntax")
        if len(fields) > base:
            if fields[base] != "moves" or len(fields) == base + 1 or any(not MOVE.fullmatch(move) for move in fields[base + 1:]):
                raise ValueError("invalid position move list")
        command = " ".join(fields)
        if command in result:
            raise ValueError("duplicate root command; repetitions belong in --repeats")
        result.append(command)
    if not result:
        raise ValueError("empty root corpus")
    return result


def configuration(side: str, workers: int, hash_mb: int, lane: str = "timing") -> dict:
    if side not in {"A", "B"} or workers not in {4, 8}:
        raise ValueError("only A/B and independently reported 4T/8T are supported")
    if lane not in {"timing", "counters"}:
        raise ValueError("lane must be timing or counters")
    mode = "LazySMP" if side == "A" else "Speculative"
    top_n = 0 if side == "A" else workers - 1
    options = {
        "Threads": str(workers), "Hash": str(hash_mb), "SearchMode": mode,
        "SpecTopN": str(top_n), "LazyFlags": "15", "MoveOverhead": "0",
        "MultiPV": "1", "FV_SCALE": "24",
        "LazyDiagnostics": str(lane == "counters" and side == "A").lower(),
        "SpeculativeDiagnostics": str(lane == "counters" and side == "B").lower(),
        "NmpDiagnostics": "false", "AspirationDiagnostics": "false", "UseBook": "false",
    }
    return {
        "side": side, "mode": mode, "options": options, "measurement_lane": lane,
        "speculative_tt_use_required": lane == "counters" and side == "B",
        "configured_search_workers": workers,
        "speculative_background_cap": top_n,
        "dedicated_speculative_pool_threads": workers if side == "B" else 0,
        "pool_topology_evidence": "derived_from_current_source_and_explicit_options; OS_threads_not_measured",
        "node_accounting": "sum_of_lazy_workers" if side == "A" else "shared_invocation_including_background",
    }


def admitted_options(handshake: list[str], requested: dict[str, str]) -> dict[str, str]:
    advertised = {}
    for line in handshake:
        if line.startswith("option name ") and " type " in line:
            name, declaration = line[len("option name "):].split(" type ", 1)
            advertised[name] = declaration
    actual = dict(requested)
    if "UseBook" not in advertised:
        actual.pop("UseBook")  # Builds without the feature cannot enable a book.
    missing = set(actual) - set(advertised)
    if missing:
        raise ValueError("unadvertised required options: " + ", ".join(sorted(missing)))
    mode_fields = advertised["SearchMode"].split()
    modes = [mode_fields[index + 1] for index, token in enumerate(mode_fields[:-1]) if token == "var"]
    if actual["SearchMode"] not in modes:
        raise ValueError("engine does not advertise requested search mode")
    return actual


def check_messages(lines: list[str]) -> None:
    for line in lines:
        lower = line.lower()
        if any(marker in lower for marker in ("cannot set ", "info string invalid ", "weight load failed", "nnue load failed", "failed to load")):
            raise ValueError("engine rejected the diagnostic configuration: " + line)
        if "weights loaded" in lower or "evaluator format" in lower:
            raise ValueError("material-only lane unexpectedly loaded an evaluator")


class TranscriptEngine:
    """One protocol reader, durable transcript, bounded waits and owned cleanup."""
    def __init__(self, binary: Path, workers: int, directory: Path, timeout: float):
        self.timeout = timeout
        self.events = (directory / "usi.jsonl").open("x", encoding="utf-8")
        self.errors = (directory / "stderr.txt").open("x", encoding="utf-8")
        self.lock = threading.Lock()
        self.responses: queue.Queue = queue.Queue()
        try:
            self.proc = subprocess.Popen(
                [str(binary)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                stderr=self.errors, text=True, bufsize=1, cwd=binary.parent,
                env=dict(os.environ, RAYON_NUM_THREADS=str(workers)),
            )
        except BaseException:
            self.events.close()
            self.errors.close()
            raise
        self.reader = threading.Thread(target=self._read, daemon=True)
        self.reader.start()

    def _record(self, direction: str, line: str) -> None:
        with self.lock:
            self.events.write(json.dumps({"direction": direction, "line": line, "monotonic_ns": time.monotonic_ns()}) + "\n")
            self.events.flush()

    def _read(self) -> None:
        try:
            assert self.proc.stdout is not None
            for line in self.proc.stdout:
                line = line.rstrip("\r\n")
                self._record("stdout", line)
                self.responses.put(line)
        except BaseException as error:
            self.responses.put(error)
        finally:
            self.responses.put(None)

    def send(self, command: str) -> None:
        self._record("stdin", command)
        assert self.proc.stdin is not None
        self.proc.stdin.write(command + "\n")
        self.proc.stdin.flush()

    def wait_for(self, token: str) -> list[str]:
        deadline = time.monotonic() + self.timeout
        lines = []
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("USI response timeout: " + token)
            try:
                line = self.responses.get(timeout=remaining)
            except queue.Empty as error:
                raise TimeoutError("USI response timeout: " + token) from error
            if line is None:
                raise RuntimeError("engine exited before " + token)
            if isinstance(line, BaseException):
                raise line
            lines.append(line)
            if line == token or (token == "bestmove" and line.startswith("bestmove ")):
                return lines

    def close(self) -> dict:
        killed = False
        try:
            self.send("quit")
        except (OSError, ValueError):
            pass
        try:
            exitcode = self.proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            killed = True
            self.proc.kill()
            exitcode = self.proc.wait()
        self.reader.join(timeout=5)
        reader_finished = not self.reader.is_alive()
        if self.proc.stdin is not None:
            self.proc.stdin.close()
        if reader_finished:
            if self.proc.stdout is not None:
                self.proc.stdout.close()
            self.events.close()
        self.errors.close()
        return {"exitcode": exitcode, "killed": killed, "reaped": True, "reader_finished": reader_finished}


def diagnostic_record(lines: list[str], tag: str, keys: tuple[str, ...]) -> dict | None:
    """One complete record per go; duplicates and partial records fail closed."""
    prefix = "info string " + tag
    records = [line[len(prefix):].split() for line in lines
               if line == prefix or line.startswith(prefix + " ")]
    if not records:
        return None
    if len(records) != 1 or len(records[0]) != 2 * len(keys):
        raise ValueError(tag + " requires one complete diagnostic record")
    parts = records[0]
    if len(set(parts[::2])) != len(keys) or set(parts[::2]) != set(keys):
        raise ValueError(tag + " has duplicate, missing or unsupported fields")
    values = dict(zip(parts[::2], parts[1::2]))
    for key, value in values.items():
        if key in LITERAL_FIELDS:
            if value != LITERAL_FIELDS[key]:
                raise ValueError("unsupported diagnostic scope: " + key)
        elif key == "deadline_overrun_ns" and value == "none":
            values[key] = None
        elif value.isascii() and value.isdecimal() and int(value) <= 2**64 - 1:
            values[key] = int(value)
        else:
            raise ValueError(tag + " contains invalid unsigned metric: " + key)
    return values


def validate_speculative(stats: dict, config: dict, nodes: int, engine_ms: int, movetime_ms: int) -> None:
    if stats["version"] != 1:
        raise ValueError("unsupported speculative diagnostic version")
    if (stats["configured_workers"] != config["configured_search_workers"]
            or stats["background_cap"] != config["speculative_background_cap"]):
        raise ValueError("speculative runtime pool count/cap differs from requested configuration")
    if (stats["candidates"] != stats["admitted"] + stats["cap_rejected"]
            or stats["admitted"] != stats["completed"] + stats["aborted"] + stats["unwound"]
            or stats["started"] + stats["unstarted"] != stats["admitted"]
            or stats["completed"] > stats["started"]
            or stats["unwound"] != 0 or stats["outstanding_after_join"] != 0
            or stats["peak_in_flight"] > min(stats["background_cap"], stats["admitted"])
            or bool(stats["peak_in_flight"]) != bool(stats["admitted"])
            or stats["background_nodes"] > nodes):
        raise ValueError("speculative task accounting/cap/join invariant failed")
    samples, total, maximum = (stats[key] for key in
        ("cancellation_latency_samples", "cancellation_latency_ns_sum", "cancellation_latency_ns_max"))
    if (samples + stats["aborted_without_observed_cancel"] != stats["aborted"]
            or (samples == 0 and (total != 0 or maximum != 0))
            or not maximum <= total <= samples * maximum
            or maximum > stats["elapsed_ns"]):
        raise ValueError("speculative cancellation interval accounting failed")
    if (stats["scope_join_wait_ns"] > stats["elapsed_ns"]
            or max(1, stats["elapsed_ns"] // 1_000_000) != engine_ms
            or stats["deadline_overrun_ns"] != max(0, stats["elapsed_ns"] - movetime_ms * 1_000_000)):
        raise ValueError("speculative deadline/elapsed/join metric mismatch")


def validate_tt_use(stats: dict) -> None:
    if stats["version"] != 1 or stats["available"] != 1:
        raise ValueError("speculative TT use requires supported, available attribution")
    cutoffs = sum(stats[key] for key in (
        "main_cutoffs_from_speculation", "quiescence_cutoffs_from_speculation",
        "slack_cutoffs_from_speculation", "probcut_cutoffs_from_speculation"))
    if (cutoffs > stats["score_hits_from_speculation"]
            or not stats["nonroot_move_first_beta_cuts"] <= stats["nonroot_move_first_completed"]
            <= stats["nonroot_move_first_started"] <= stats["move_hits_from_speculation"]):
        raise ValueError("speculative TT use event counts exceed their observed hits/starts")


def parse_result(lines: list[str], elapsed_ms: float, config: dict, movetime_ms: int) -> dict:
    check_messages(lines)
    best = [line.split()[1] for line in lines if line.startswith("bestmove ")]
    if len(best) != 1 or not MOVE.fullmatch(best[0]):
        raise ValueError("diagnostic corpus requires one ordinary bestmove per nonterminal root")
    result = parse_search_result(lines)
    if len(result["score"]) not in {2, 3} or result["score"][0] not in {"cp", "mate"}:
        raise ValueError("invalid score kind or bound")
    int(result["score"][1])
    if any(not MOVE.fullmatch(move) for move in result["pv"]):
        raise ValueError("malformed PV move")
    if result["pv"] and result["pv"][0] != best[0]:
        raise ValueError("bestmove differs from the returned primary PV")
    primary = next(line.split() for line in reversed(lines)
                   if line.startswith("info ") and " score " in line and " nodes " in line
                   and (" multipv " not in line or " multipv 1 " in line))
    def number(key):
        value = int(primary[primary.index(key) + 1])
        if value < 0:
            raise ValueError("negative search metric: " + key)
        return value
    depth, nodes, engine_ms = (number(key) for key in ("depth", "nodes", "time"))
    lazy = parse_lazy_smp(lines)
    if config["mode"] == "LazySMP":
        if lazy is None or lazy.get("workers") != config["configured_search_workers"]:
            raise ValueError("Lazy SMP runtime worker report differs from requested worker count")
        workers = lazy.get("worker_stats", [])
        if ([worker["worker"] for worker in workers] != list(range(config["configured_search_workers"]))
                or sum(worker["nodes"] for worker in workers) != nodes
                or sum(worker["selected"] for worker in workers) != 1
                or not 0 <= lazy["selected_worker"] < len(workers)
                or not workers[lazy["selected_worker"]]["selected"]):
            raise ValueError("Lazy SMP worker records are missing or inconsistent")
    elif lazy is not None:
        raise ValueError("Speculative slot unexpectedly emitted Lazy SMP results")
    lane = config["measurement_lane"]
    spec = diagnostic_record(lines, "speculative_diagnostics", SPEC_FIELDS)
    tt_use = diagnostic_record(lines, "speculative_tt_use", TT_USE_FIELDS)
    lazy_tt = diagnostic_record(lines, "lazy_smp_tt", LAZY_TT_FIELDS)
    lazy_overlap = diagnostic_record(lines, "lazy_smp_overlap", LAZY_OVERLAP_FIELDS)
    if lane == "timing":
        if any(record is not None for record in (spec, tt_use, lazy_tt, lazy_overlap)):
            raise ValueError("timing lane unexpectedly emitted opt-in counter diagnostics")
    elif config["mode"] == "Speculative":
        if spec is None or lazy_tt is not None or lazy_overlap is not None:
            raise ValueError("counter lane requires speculative diagnostics only for B")
        validate_speculative(spec, config, nodes, engine_ms, movetime_ms)
        # Only legacy recorded configurations may omit the new record. An
        # absent historical measurement remains None, never a fabricated zero.
        if tt_use is None and config.get("speculative_tt_use_required", False):
            raise ValueError("counter lane requires speculative_tt_use version 1 for B")
        if tt_use is not None:
            validate_tt_use(tt_use)
    else:
        if spec is not None or tt_use is not None or lazy_tt is None or lazy_overlap is None:
            raise ValueError("counter lane requires Lazy SMP TT and overlap diagnostics for A")
        if (lazy_tt["hits"] > lazy_tt["probes"] or lazy_tt["committed"] > lazy_tt["attempted"]
                or lazy_overlap["sample_rate"] != 256
                or lazy_overlap["tt_local_hits"] + lazy_overlap["tt_foreign_hits"] + lazy_overlap["tt_unknown_hits"] != lazy_overlap["tt_sampled_hits"]):
            raise ValueError("Lazy SMP counter metrics are inconsistent")
    return {
        **result, "bestmove": best[0], "depth": depth, "nodes": nodes,
        "completed_iteration": depth > 0, "engine_elapsed_ms": engine_ms,
        "elapsed_ms": elapsed_ms, "wall_nps": nodes * 1000 / elapsed_ms if lane == "timing" and elapsed_ms > 0 else None,
        "response_overrun_ms": max(0.0, elapsed_ms - movetime_ms),
        "engine_overrun_ms": max(0, engine_ms - movetime_ms),
        "lazy_smp": lazy, "pv_legality_independently_verified": False,
        "speculative_runtime_pool_count_observed": spec is not None,
        "measurement_lane": lane, "performance_comparison_eligible": lane == "timing",
        "speculative_diagnostics": spec, "lazy_smp_tt": lazy_tt, "lazy_smp_overlap": lazy_overlap,
        "speculative_tt_use": tt_use, "tt_publication_use_observed": tt_use is not None,
        "useful_tt_cutoff_attribution": "not_measured",
    }


def run_slot(binary: Path, root: str, config: dict, movetime_ms: int, timeout: float, directory: Path) -> dict:
    engine = TranscriptEngine(binary, config["configured_search_workers"], directory, timeout)
    try:
        engine.send("usi")
        handshake = engine.wait_for("usiok")
        check_messages(handshake)
        actual = admitted_options(handshake, config["options"])
        for name, value in actual.items():
            engine.send(f"setoption name {name} value {value}")
        engine.send("isready")
        check_messages(engine.wait_for("readyok"))
        engine.send("usinewgame")
        engine.send("isready")
        check_messages(engine.wait_for("readyok"))
        engine.send("position " + root)
        started = time.perf_counter()
        engine.send(f"go movetime {movetime_ms}")
        lines = engine.wait_for("bestmove")
        elapsed = (time.perf_counter() - started) * 1000
        result = parse_result(lines, elapsed, config, movetime_ms)
        result["effective_options"] = actual
    finally:
        shutdown = engine.close()
        save_json(directory / "shutdown.json", shutdown)
    if shutdown != {"exitcode": 0, "killed": False, "reaped": True, "reader_finished": True}:
        raise ValueError("engine did not exit normally; see shutdown.json")
    check_messages((directory / "stderr.txt").read_text(encoding="utf-8").splitlines())
    return {**result, "shutdown": shutdown}


def summarize(rows: list[dict], expected: list[tuple], lane: str = "timing") -> dict:
    identities = [(row["repeat"], row["workers"], row["position"], row["abba_slot"], row["side"]) for row in rows]
    if identities != expected:
        raise ValueError("missing, reordered or duplicated ABBA slots; incomplete evidence cannot be summarized")
    if any(row.get("measurement_lane", "timing") != lane for row in rows):
        raise ValueError("measurement lanes must not be mixed")
    grouped = {}
    for index in range(0, len(rows), 4):
        block = rows[index:index + 4]
        if [row["side"] for row in block] != list("ABBA") or len({row["root_sha256"] for row in block}) != 1:
            raise ValueError("ABBA root mismatch")
        if lane == "counters":
            grouped.setdefault(str(block[0]["workers"]), []).extend(block)
            continue
        means = {side: {key: statistics.mean(row[key] for row in block if row["side"] == side)
                        for key in ("depth", "elapsed_ms", "nodes", "response_overrun_ms")}
                 for side in "AB"}
        delta = {key: means["B"][key] - means["A"][key] for key in means["A"]}
        grouped.setdefault(str(block[0]["workers"]), []).append(delta)
    if lane == "counters":
        reports = {}
        for workers, observations in grouped.items():
            speculative = [row["speculative_diagnostics"] for row in observations if row["side"] == "B"]
            lazy_tt = [row["lazy_smp_tt"] for row in observations if row["side"] == "A"]
            lazy_overlap = [row["lazy_smp_overlap"] for row in observations if row["side"] == "A"]
            tt_use = [row.get("speculative_tt_use") for row in observations if row["side"] == "B"]
            reports[workers] = {
                "complete_blocks": len(observations) // 4,
                "speculative_totals": {key: sum(row[key] for row in speculative) for key in (
                    "candidates", "admitted", "started", "completed", "aborted", "unstarted", "cap_rejected", "background_nodes",
                    "cancellation_latency_samples", "cancellation_latency_ns_sum", "aborted_without_observed_cancel")},
                "speculative_maxima": {key: max(row[key] for row in speculative) for key in (
                    "peak_in_flight", "scope_join_wait_ns", "deadline_overrun_ns", "cancellation_latency_ns_max")},
                "lazy_tt_totals": {key: sum(row[key] for row in lazy_tt) for key in LAZY_TT_FIELDS},
                "lazy_overlap_totals": {key: sum(row[key] for row in lazy_overlap) for key in LAZY_OVERLAP_FIELDS if key != "sample_rate"},
                "speculative_tt_use_coverage": {"observed_slots": sum(row is not None for row in tt_use), "total_slots": len(tt_use)},
                "speculative_tt_use_totals": {key: sum(row[key] for row in tt_use) for key in TT_USE_COUNTERS} if all(row is not None for row in tt_use) else None,
            }
        return {
            "schema": SCHEMA + ".summary", "measurement_lane": lane, "diagnostic_only": True,
            "strength_gain_proven": False, "performance_comparison_eligible": False,
            "interpretation": "Separate task/exact TT-use counters (B) and per-node/sampled TT counters (A). Different observer overheads prohibit performance inference from time or node differences. Exact TT-use events prove captured-bound/move consumption, not counterfactual savings. Foreign TT hits are not proven useful cutoffs.",
            "by_workers": reports,
        }
    return {
        "schema": SCHEMA + ".summary", "measurement_lane": lane, "diagnostic_only": True, "strength_gain_proven": False,
        "performance_comparison_eligible": False,
        "interpretation": "B minus A within each root/repeat ABBA block; depth and node counts do not establish move quality or Elo",
        "by_workers": {workers: {"complete_blocks": len(blocks), "median_paired_delta": {
            key: statistics.median(block[key] for block in blocks) for key in blocks[0]
        }} for workers, blocks in grouped.items()},
    }


def resource_snapshot(resource_root: Path, output: Path, minimum_free_mib: int) -> dict:
    paths = {"resource_root": resource_root, "output": output}
    samples = {}
    for label, path in paths.items():
        try:
            if not path.is_dir():
                raise ValueError("required directory is absent")
            free = shutil.disk_usage(path).free
            samples[label] = {"path": str(path.resolve()), "free_bytes": free, "admitted": free >= minimum_free_mib * 1024**2}
        except (OSError, ValueError) as error:
            samples[label] = {"path": str(path), "admitted": False, "error": str(error)}
    return {"minimum_free_mib": minimum_free_mib, "admitted": all(row["admitted"] for row in samples.values()), "paths": samples}


def host_snapshot_admitted(snapshot: dict, ceiling_percent: int) -> bool:
    """Relax only the per-process CPU ceiling; reject unknown observations."""
    if idle_snapshot_state(snapshot) == "unknown":
        return False
    return all(process["cpu_percent"] < ceiling_percent for process in snapshot["busy_processes"])


def verify_frozen_inputs(manifest: dict, output: Path) -> None:
    if (sha256(Path(manifest["engine"]["path"])) != manifest["engine"]["sha256"]
            or sha256(output / "positions.input") != manifest["positions_sha256"]):
        raise ValueError("frozen inputs changed")
    if manifest.get("legal_moves") is not None:
        tool = manifest["legal_moves"]
        if sha256(Path(tool["path"])) != tool["sha256"]:
            raise ValueError("frozen legal-move tool changed")
    for name, expected_digest in manifest["measurement_source_sha256"].items():
        if sha256(Path(__file__).parent / name) != expected_digest:
            raise ValueError("measurement driver changed: " + name)


def configuration_digest(manifest: dict) -> str:
    keys = ("configurations", "roots", "repeats", "budget", "evaluator", "measurement_lane", "worker_order")
    if "host_admission_policy" in manifest:
        keys += ("host_admission_policy",)
    encoded = json.dumps({key: manifest[key] for key in keys}, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(encoded).hexdigest()


def validate_completed_run(manifest: dict, output: Path) -> None:
    """Only a present, successful receipt for these exact bytes admits completion."""
    from validate_paired_pv import SCHEMA as PV_SCHEMA, validate_runs
    report_path = output / "pv-validation.json"
    stage_hash = sha256(output / "manifest.json")
    manifest["pv_validation"] = {"status": "error", "valid": False, "path": str(report_path.resolve())}
    try:
        returned = validate_runs(Path(manifest["legal_moves"]["path"]), report_path, [output], allow_pending=True)
    finally:
        if report_path.is_file():
            manifest["pv_validation"]["sha256"] = sha256(report_path)
    report = json.loads(report_path.read_bytes())
    if (report != returned or report.get("schema") != PV_SCHEMA or report.get("status") != "complete"
            or report.get("valid") is not True or report.get("violations") != [] or report.get("errors") != []):
        raise ValueError("legal-PV validation failed, unknown, or incomplete; see pv-validation.json")
    records = report.get("runs")
    if not isinstance(records, list) or len(records) != 1:
        raise ValueError("legal-PV validation lacks the exact run receipt")
    record = records[0]
    rows = [json.loads(line) for line in (output / "rows.jsonl").read_text().splitlines()]
    expected_history_moves = sum(len(roots.split(" moves ", 1)[1].split()) if " moves " in roots else 0
                                 for roots in (manifest["roots"][row["position"]] for row in rows))
    if (record.get("path") != str(output.resolve()) or record.get("strict_row_identity_verified") is not True
            or record.get("source_manifest_sha256") != stage_hash
            or sha256(Path(record["source_manifest_snapshot"])) != stage_hash
            or sha256(output / "manifest.json") != stage_hash
            or record.get("rows_sha256") != sha256(output / "rows.jsonl")
            or record.get("corpus_sha256") != manifest["positions_sha256"]
            or record.get("engine_sha256") != {side: manifest["engine"]["sha256"] for side in "AB"}
            or record.get("configuration_sha256") != manifest["configuration_sha256"]
            or record.get("rows") != manifest["expected_searches"]
            or report.get("rows") != manifest["expected_searches"]
            or report.get("pv_moves") != sum(len(row["pv"]) for row in rows)
            or report.get("root_history_moves") != expected_history_moves
            or report.get("legal_tool_sha256") != manifest["legal_moves"]["sha256"]
            or sha256(Path(report["legal_tool_frozen"])) != manifest["legal_moves"]["sha256"]
            or sha256(report_path) != manifest["pv_validation"]["sha256"]):
        raise ValueError("legal-PV validation identity/count binding failed")
    verify_frozen_inputs(manifest, output)
    manifest["pv_validation"].update(status="complete", valid=True, rows=report["rows"],
                                     pv_moves=report["pv_moves"], root_history_moves=report["root_history_moves"])


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--positions", type=Path, required=True)
    parser.add_argument("--legal-moves", type=Path, help="Sekirei legal-move binary; required before engines can run")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--threads", default="4")
    parser.add_argument("--lane", choices=("timing", "counters"), default="timing")
    parser.add_argument("--repeats", type=int, default=2)
    parser.add_argument("--movetime", type=int, default=300)
    parser.add_argument("--hash", type=int, default=64)
    parser.add_argument("--timeout", type=float, default=60)
    parser.add_argument("--resource-root", type=Path, default=Path("/Volumes/ESD-EPA/Projects/sekirei_data"))
    parser.add_argument("--minimum-free-mib", type=int, default=2048)
    parser.add_argument("--host-process-ceiling-percent", type=int, choices=(25, 50, 75, 100), default=25,
                        help="per-process CPU admission ceiling; values above 25 are exploratory and cannot qualify timing")
    parser.add_argument("--source-ref")
    parser.add_argument("--prepare-only", action="store_true")
    args = parser.parse_args(argv)
    try:
        workers = [int(value) for value in args.threads.split(",")]
        if not workers or any(value not in {4, 8} for value in workers) or len(set(workers)) != len(workers):
            raise ValueError("threads must be 4, 8, or 4,8; each is reported independently")
        if min(args.repeats, args.movetime, args.hash, args.minimum_free_mib) < 1 or args.hash > 2048:
            raise ValueError("positive repeats/time/capacity and Hash 1..2048 are required")
        if not math.isfinite(args.timeout) or args.timeout <= args.movetime / 1000:
            raise ValueError("response timeout must be finite and exceed the search budget")
        source = args.positions.read_bytes()
        roots = positions_from_text(source.decode("utf-8"))
    except (OSError, UnicodeError, ValueError) as error:
        parser.error(str(error))
    args.output.mkdir(parents=True, exist_ok=False)
    binary = (args.output / "engine").resolve()
    digest = freeze(args.binary, binary)
    legal_moves = None
    if args.legal_moves is not None:
        legal_path = (args.output / "legal-moves").resolve()
        legal_moves = {"path": str(legal_path), "sha256": freeze(args.legal_moves, legal_path),
                       "source_path": str(args.legal_moves.resolve()), "kind": "same-rules Sekirei checker",
                       "build_identity_verified": False}
    (args.output / "positions.input").write_bytes(source)
    configs = {str(count): {side: configuration(side, count, args.hash, args.lane) for side in "AB"} for count in workers}
    expected = list(schedule(workers, len(roots), args.repeats))
    manifest = {
        "schema": SCHEMA + ".manifest", "status": "prepared", "diagnostic_only": True,
        "strength_gate_opened": False, "default_adoption_authorized": False,
        "started_at_utc": datetime.now(timezone.utc).isoformat(),
        "engine": {"path": str(binary), "sha256": digest, "source_path": str(args.binary.resolve())},
        "legal_moves": legal_moves,
        "pv_validation": {"status": "not_run", "valid": False},
        "timing_qualification_contract": "v2: complete ABBA, frozen inputs and every row identity verified, same-rules legal root-history/PV replay passed, timing lane, strict 25-percent host admission and idle post-run snapshot; no strength or independent-oracle claim",
        "source_ref": args.source_ref, "source_ref_verified": False,
        "positions_sha256": hashlib.sha256(source).hexdigest(), "roots": roots,
        "root_freshness": "fresh process and usinewgame every slot; corpus novelty and independent-holdout status not certified",
        "configurations": configs, "repeats": args.repeats, "worker_order": workers,
        "budget": {"kind": "equal_wall_time", "go_movetime_ms": args.movetime, "move_overhead_ms": 0, "node_limit": None},
        "evaluator": {"kind": "material-only", "engine_argument": None, "EvalFile_sent": False},
        "measurement_lane": args.lane, "counter_lane": args.lane == "counters", "timing_qualified": False,
        "host_admission_policy": {"observer_floor_percent": 25,
                                  "per_process_ceiling_percent": args.host_process_ceiling_percent,
                                  "strict_timing_eligible": args.host_process_ceiling_percent == 25},
        "host_admission_observed": {"sampled_slots": 0, "non_idle_slots": 0,
                                    "max_observed_busy_process_cpu_percent": 0.0},
        "observer_contract": "counters: A uses Lazy SMP per-node/sampled TT observers; B uses speculative task-boundary observers; their time and node differences are not comparable performance evidence",
        "foreign_tt_hit_contract": "sampled foreign-origin observation only; not proven useful cutoff or research avoided",
        "speculative_tt_use_contract": "current invocation speculative score/move origins captured with the TT publication; actual main/QS/slack/Small ProbCut returns and non-root first-child events; no counterfactual savings or root-order attribution",
        "schedule": "per root and worker count ABBA; fresh process every slot", "expected_searches": len(expected),
        "completed_searches": 0, "response_timeout_seconds": args.timeout,
        "resource_root": str(args.resource_root), "minimum_free_mib": args.minimum_free_mib,
        "measurement_source_sha256": {name: sha256(Path(__file__).parent / name) for name in SOURCES},
        "host": {"system": platform.system(), "release": platform.release(), "machine": platform.machine(), "logical_cpus": os.cpu_count(), "physical_cores_verified": None},
        "physical_scaling_claim_qualified": False,
        "limitations": ["Legal root-history/PV replay uses a frozen caller-supplied Sekirei legal-move binary; it is not an independent rule oracle, build-provenance proof, repetition adjudication, or mate-score proof", "Speculative TT-use attribution is available only in the counter lane and does not measure counterfactual saved work", "No move-quality or playing-strength inference", "Relaxed host admission records contention and cannot qualify timing", "Host snapshots do not exclude transient contention", "Configured workers do not count idle or auxiliary OS threads", "Physical core count and oversubscription require separate verification"],
    }
    manifest["configuration_sha256"] = configuration_digest(manifest)
    save_json(args.output / "manifest.json", manifest)
    if args.prepare_only:
        return 0
    if legal_moves is None:
        manifest.update(status="blocked_validation", error="--legal-moves is required before execution")
        save_json(args.output / "manifest.json", manifest)
        return 2
    rows = []
    try:
        with (args.output / "rows.jsonl").open("x", encoding="utf-8") as stream:
            for repeat, count, position, slot, side in expected:
                resources = resource_snapshot(args.resource_root, args.output, args.minimum_free_mib)
                activity = host_activity(strict=True)
                host_accepted = host_snapshot_admitted(activity, args.host_process_ceiling_percent)
                manifest["last_admission"] = {"sequence": len(rows), "resources": resources, "host_activity": activity,
                                              "host_accepted": host_accepted}
                observed = manifest["host_admission_observed"]
                observed["sampled_slots"] += 1
                observed["non_idle_slots"] += idle_snapshot_state(activity) == "busy"
                observed["max_observed_busy_process_cpu_percent"] = max(
                    [observed["max_observed_busy_process_cpu_percent"]]
                    + [process["cpu_percent"] for process in (activity.get("busy_processes") or [])]
                )
                if not resources["admitted"] or not host_accepted:
                    manifest.update(status="blocked_admission" if not rows else "incomplete", timing_qualified=False)
                    save_json(args.output / "manifest.json", manifest)
                    return 2
                verify_frozen_inputs(manifest, args.output)
                manifest["status"] = "running"
                save_json(args.output / "manifest.json", manifest)
                directory = args.output / f"slot-{len(rows):05d}"
                directory.mkdir()
                config = configs[str(count)][side]
                save_json(directory / "request.json", {"configuration": config, "root": roots[position], "admission": manifest["last_admission"]})
                row = run_slot(binary, roots[position], config, args.movetime, args.timeout, directory)
                row.update(schema=SCHEMA + ".row", measurement_lane=args.lane, repeat=repeat, workers=count, position=position,
                           abba_slot=slot, side=side, sequence=len(rows), engine_sha256=digest,
                           root_sha256=hashlib.sha256(roots[position].encode()).hexdigest(),
                           configuration_sha256=manifest["configuration_sha256"], artifact_directory=directory.name)
                rows.append(row)
                stream.write(json.dumps(row, allow_nan=False) + "\n")
                stream.flush()
                os.fsync(stream.fileno())
                manifest["completed_searches"] = len(rows)
                save_json(args.output / "manifest.json", manifest)
        after = host_activity(strict=True)
        manifest["host_activity_after"] = after
        verify_frozen_inputs(manifest, args.output)
        summary = summarize(rows, expected, args.lane)
        save_json(args.output / "summary.json", summary)
        manifest["status"] = "validating"
        save_json(args.output / "manifest.json", manifest)
        validate_completed_run(manifest, args.output)
        manifest.update(status="complete", timing_qualified=(args.lane == "timing"
                         and args.host_process_ceiling_percent == 25 and idle_snapshot_state(after) == "idle"))
        summary.update(performance_comparison_eligible=manifest["timing_qualified"],
                       pv_validation_sha256=manifest["pv_validation"]["sha256"])
        save_json(args.output / "summary.json", summary)
        save_json(args.output / "manifest.json", manifest)
        return 0
    except BaseException as error:
        manifest.update(status="incomplete", timing_qualified=False, error=f"{type(error).__name__}: {error}")
        save_json(args.output / "manifest.json", manifest)
        raise


if __name__ == "__main__":
    raise SystemExit(main())
