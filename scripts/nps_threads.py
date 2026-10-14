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
import hashlib
import json
import os
import platform
import queue
import threading
import statistics
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ROOT_HINT_FLAG = 2048
ROOT_HINT_KEYS = ("publications", "eligible_reads", "applied")
ROOT_HINT_TRANSFER_KEYS = ROOT_HINT_KEYS + (
    "first_move_beta_cuts", "researches", "research_nodes", "stale_research_nodes", "stale_reads",
    "depth_gap_sum", "depth_gap_max", "generation_gap_sum", "generation_gap_max",
)


class Usi:
    def __init__(self, binary: str, env: dict[str, str], timeout: float = 120):
        self.timeout = timeout
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
        responses: queue.Queue = queue.Queue(maxsize=1)

        def read_response():
            lines = []
            try:
                while True:
                    line = self.proc.stdout.readline()
                    if not line:
                        raise RuntimeError("engine exited")
                    line = line.rstrip("\n")
                    lines.append(line)
                    if line.startswith(token):
                        responses.put(lines)
                        return
            except BaseException as error:
                responses.put(error)

        # A blocked text readline cannot enforce deadlines itself. The caller's
        # finally block kills/reaps the process after a timeout, releasing this
        # reader too. Only one reader exists per protocol response.
        reader = threading.Thread(target=read_response, daemon=True)
        reader.start()
        try:
            result = responses.get(timeout=self.timeout)
        except queue.Empty as error:
            raise TimeoutError(f"engine did not emit {token!r} within {self.timeout}s") from error
        if isinstance(result, BaseException):
            raise result
        reader.join()
        return result

    def quit(self) -> None:
        try:
            self.send("quit")
        except Exception:
            pass
        try:
            self.proc.wait(timeout=5)
        except Exception:
            # `kill` alone leaves a zombie until the Python runner exits.
            # Always wait after the fallback termination as well.
            try:
                self.proc.kill()
            finally:
                self.proc.wait()


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
                "discarded_root_nodes": int(
                    parts[parts.index("discarded_root_nodes") + 1]
                ),
                "divergent_discarded_nodes": int(
                    parts[parts.index("divergent_discarded_nodes") + 1]
                ),
                "stop_lag_ms": stop_lag_ms,
            }
            selected_reuse_keys = (
                "selected_tt_local_hits",
                "selected_tt_foreign_hits",
                "selected_tt_unknown_hits",
            )
            if all(key in parts for key in selected_reuse_keys):
                result.update(
                    {
                        key: int(parts[parts.index(key) + 1])
                        for key in selected_reuse_keys
                    }
                )
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
        if len(fields) not in (10, 13) or not fields[0].startswith("w"):
            return []
        worker_label = fields[0]
        selected = worker_label.endswith("*")
        try:
            worker = {
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
            if len(fields) == 13:
                worker.update(
                    {
                        "tt_local_hits": int(fields[10].removeprefix("l")),
                        "tt_foreign_hits": int(fields[11].removeprefix("f")),
                        "tt_unknown_hits": int(fields[12].removeprefix("u")),
                    }
                )
            workers.append(worker)
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


def parse_lazy_smp_overlap(lines: list[str]) -> dict[str, int] | None:
    """Sampled cross-worker position overlap from the last search."""
    prefix = "info string lazy_smp_overlap "
    keys = (
        "sample_rate",
        "visits",
        "unique",
        "within_worker_revisits",
        "cross_worker_visits",
        "shared_fingerprints",
        "dropped",
        "tt_sampled_stores",
        "tt_sampled_hits",
        "tt_local_hits",
        "tt_foreign_hits",
        "tt_unknown_hits",
        "tt_dropped_stores",
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


def parse_lazy_smp_root_hint(
    lines: list[str], *, required: bool = False
) -> dict[str, int] | None:
    """Parse one invocation-scoped helper-root-hint diagnostic.

    Unlike the optional legacy diagnostics, a requested root-hint lane fails
    closed.  A missing, duplicate, malformed, negative, or internally
    inconsistent record must not silently become a zero-use measurement.
    """
    prefix = "info string lazy_smp_root_hint "
    matching = [line for line in lines if line.startswith(prefix)]
    if not matching:
        if required:
            raise ValueError("missing lazy_smp_root_hint diagnostic")
        return None
    if len(matching) != 1:
        raise ValueError(
            f"expected one lazy_smp_root_hint diagnostic, got {len(matching)}"
        )

    parts = matching[0][len(prefix) :].split()
    keys = ROOT_HINT_TRANSFER_KEYS if len(parts) > 6 else ROOT_HINT_KEYS
    if len(parts) != 2 * len(keys) or parts[::2] != list(keys):
        raise ValueError(
            f"malformed lazy_smp_root_hint diagnostic: {matching[0]!r}"
        )
    raw_values = parts[1::2]
    if any(not value.isascii() or not value.isdigit() for value in raw_values):
        raise ValueError("lazy_smp_root_hint counters must be unsigned decimal integers")
    values = dict(zip(keys, map(int, raw_values), strict=True))
    if any(value > (1 << 64) - 1 for value in values.values()):
        raise ValueError("lazy_smp_root_hint counter exceeds u64")
    if values["applied"] > values["eligible_reads"]:
        raise ValueError("root-hint applications exceed eligible reads")
    if values["publications"] == 0 and (
        values["eligible_reads"] != 0 or values["applied"] != 0
    ):
        raise ValueError("root-hint reads exist without a publication")
    if "first_move_beta_cuts" in values:
        if values["first_move_beta_cuts"] > values["applied"]:
            raise ValueError("root-hint first-move cuts exceed applications")
        for kind in ("depth", "generation"):
            if values[f"{kind}_gap_max"] > values[f"{kind}_gap_sum"]:
                raise ValueError(f"root-hint {kind} maximum exceeds sum")
        if values["generation_gap_sum"]:
            raise ValueError("root-hint consumed across invocation generations")
    return values


def parse_search_result(lines: list[str]) -> dict:
    """Use the final primary-PV info record, keeping score units and bounds."""
    for line in reversed(lines):
        parts = line.split()
        if not parts or parts[0] != "info" or "nodes" not in parts or "score" not in parts:
            continue
        if "multipv" in parts and parts[parts.index("multipv") + 1] != "1":
            continue
        score_at = parts.index("score")
        score = parts[score_at + 1 : score_at + 3]
        for bound in ("lowerbound", "upperbound"):
            if bound in parts:
                score.append(bound)
        return {"score": score, "pv": parts[parts.index("pv") + 1 :] if "pv" in parts else []}
    raise ValueError("missing scored primary-PV search info")


def parse_retirement(lines: list[str]) -> list[dict[str, int]]:
    prefix = "info string lazy_smp_retirement "
    keys = ("worker", "completed_iterations", "retired", "foreign_contributions")
    records = []
    for line in lines:
        if not line.startswith(prefix):
            continue
        parts = line[len(prefix):].split()
        if len(parts) != 8 or parts[::2] != list(keys):
            raise ValueError("malformed retirement diagnostic")
        if any(not value.isascii() or not value.isdigit() for value in parts[1::2]):
            raise ValueError("retirement counters must be unsigned decimal integers")
        record = dict(zip(keys, map(int, parts[1::2]), strict=True))
        if any(value > (1 << 64) - 1 for value in record.values()):
            raise ValueError("retirement counter exceeds u64")
        if record["retired"] not in (0, 1) or (record["worker"] == 0 and record["retired"]):
            raise ValueError("invalid retirement state")
        if any(old["worker"] == record["worker"] for old in records):
            raise ValueError("duplicate retirement worker")
        records.append(record)
    return records


def parse_cooperative_root(lines: list[str], *, total_nodes: int | None = None) -> dict[str, int] | None:
    prefix = "info string cooperative_root "
    matches = [line[len(prefix):] for line in lines if line.startswith(prefix)]
    if not matches:
        return None
    keys = ("generations", "helper_nodes", "helper_probes", "helper_fail_highs",
            "main_confirmations", "confirmed_alpha_raises",
            "main_nodes",
            "main_tasks",
            "confirmation_nodes",
            "helper_completed_nodes",
            "helper_aborted_nodes",
            "helper_aborted_probes",
            "spawn_failures",
            "helpers_started",
            "helper_start_delay_us",
            "join_wait_us",
            "alpha_gap_sum",
            "alpha_gap_max",
            )
    fields = matches[0].split()
    if len(matches) != 1 or len(fields) != len(keys) * 2 or fields[::2] != list(keys):
        raise ValueError("malformed cooperative root diagnostic")
    if any(not value.isascii() or not value.isdigit() for value in fields[1::2]):
        raise ValueError("cooperative root counters must be unsigned decimal integers")
    result = dict(zip(keys, map(int, fields[1::2]), strict=True))
    if any(value > (1 << 64) - 1 for value in result.values()):
        raise ValueError("cooperative root counter exceeds u64")
    if result["confirmed_alpha_raises"] > result["main_confirmations"]:
        raise ValueError("cooperative raises exceed main confirmations")
    if result["main_confirmations"] > result["helper_fail_highs"] or result["helper_fail_highs"] > result["helper_probes"]:
        raise ValueError("cooperative confirmation/probe counts inconsistent")
    if result["confirmation_nodes"] > result["main_nodes"]:
        raise ValueError("cooperative confirmation nodes exceed main nodes")
    if result["helper_completed_nodes"] + result["helper_aborted_nodes"] != result["helper_nodes"]:
        raise ValueError("cooperative helper node accounting inconsistent")
    if result["alpha_gap_max"] > result["alpha_gap_sum"]:
        raise ValueError("cooperative alpha gap inconsistent")
    if result["spawn_failures"] + result["helpers_started"] != result["generations"] or result["helper_aborted_probes"] > result["helpers_started"]:
        raise ValueError("cooperative lifecycle counts inconsistent")
    if result["main_confirmations"] == 0 and result["alpha_gap_sum"] != 0:
        raise ValueError("cooperative alpha gap without confirmations")
    if result["helpers_started"] == 0 and any(result[key] != 0 for key in (
        "helper_probes", "helper_nodes", "helper_start_delay_us", "join_wait_us",
    )):
        raise ValueError("cooperative work without started helpers")
    if total_nodes is not None and result["main_nodes"] + result["helper_nodes"] > total_nodes:
        raise ValueError("cooperative classified nodes exceed total nodes")
    return result


def parse_nmp(lines: list[str]) -> list[dict]:
    """Require complete depth histograms for every emitted NMP summary."""
    keys = ("worker", "attempts", "fail_high", "verification_attempts",
            "verification_successes", "verification_overturns", "pv_returns",
            "tracking_overflow", "prior_reductions")
    workers = {}
    for line in lines:
        prefix = "info string v2_nmp "
        if not line.startswith(prefix):
            continue
        fields = line[len(prefix):].split()
        if len(fields) != len(keys) * 2 or fields[::2] != list(keys):
            raise ValueError("malformed NMP diagnostic")
        if any(not value.isascii() or not value.isdigit() for value in fields[1::2]):
            raise ValueError("NMP counters must be unsigned decimal integers")
        record = dict(zip(keys, map(int, fields[1::2]), strict=True))
        if any(value > (1 << 64) - 1 for value in record.values()):
            raise ValueError("NMP counter exceeds u64")
        if record["worker"] in workers:
            raise ValueError("duplicate NMP worker summary")
        record["by_depth"] = [[0] * 7 for _ in range(32)]
        workers[record["worker"]] = record
    seen = set()
    for line in lines:
        prefix = "info string v2_nmp_depth "
        if not line.startswith(prefix):
            continue
        fields = line[len(prefix):].split()
        if len(fields) != 6 or fields[::2] != ["worker", "depth", "counts"]:
            raise ValueError("malformed NMP depth diagnostic")
        values = fields[1:4:2] + fields[5].split(",")
        if len(values) != 9 or any(not value.isascii() or not value.isdigit() for value in values):
            raise ValueError("invalid NMP depth counter")
        worker, depth, *counts = map(int, values)
        if any(value > (1 << 64) - 1 for value in counts):
            raise ValueError("NMP depth counter exceeds u64")
        if worker not in workers or depth > 31 or (worker, depth) in seen:
            raise ValueError("invalid or duplicate NMP depth bucket")
        seen.add((worker, depth))
        workers[worker]["by_depth"][depth] = counts
    for record in workers.values():
        for column, key in enumerate(keys[1:8]):
            if sum(counts[column] for counts in record["by_depth"]) != record[key]:
                raise ValueError(f"NMP histogram does not match {key}")
        if record["fail_high"] > record["attempts"] or record["prior_reductions"] > record["attempts"]:
            raise ValueError("NMP outcomes exceed attempts")
        if record["verification_attempts"] > record["fail_high"]:
            raise ValueError("NMP verification attempts exceed fail-highs")
        if record["verification_successes"] + record["verification_overturns"] > record["verification_attempts"]:
            raise ValueError("NMP verification outcomes exceed attempts")
    return [workers[key] for key in sorted(workers)]


def parse_bestmove(lines: list[str]) -> str:
    for line in reversed(lines):
        if line.startswith("bestmove "):
            return line.split(maxsplit=2)[1]
    return ""


def require_lazy_diagnostic_record(
    lazy_smp: dict[str, object] | None,
    lazy_smp_tt: dict[str, int] | None,
    lazy_smp_overlap: dict[str, int] | None,
) -> None:
    """Fail closed when a requested Lazy SMP diagnostic record is incomplete."""
    missing = []
    if lazy_smp is None:
        missing.append("lazy_smp")
    else:
        if not lazy_smp.get("worker_stats"):
            missing.append("lazy_smp.worker_stats")
        for key in (
            "selected_tt_local_hits",
            "selected_tt_foreign_hits",
            "selected_tt_unknown_hits",
        ):
            if key not in lazy_smp:
                missing.append(f"lazy_smp.{key}")
    if lazy_smp_tt is None:
        missing.append("lazy_smp_tt")
    if lazy_smp_overlap is None:
        missing.append("lazy_smp_overlap")
    if missing:
        raise ValueError("missing required Lazy SMP diagnostics: " + ", ".join(missing))


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


def summarize_lazy_overlap(rows: list[dict]) -> dict[str, float | int] | None:
    diagnostics = [row.get("lazy_smp_overlap") for row in rows]
    diagnostics = [item for item in diagnostics if item is not None]
    if not diagnostics:
        return None
    totals = {
        key: sum(item[key] for item in diagnostics)
        for key in (
            "visits",
            "unique",
            "within_worker_revisits",
            "cross_worker_visits",
            "shared_fingerprints",
            "dropped",
            "tt_sampled_stores",
            "tt_sampled_hits",
            "tt_local_hits",
            "tt_foreign_hits",
            "tt_unknown_hits",
            "tt_dropped_stores",
        )
    }
    totals["sample_rate"] = diagnostics[0]["sample_rate"]
    totals["shared_fingerprint_rate"] = totals["shared_fingerprints"] / max(
        totals["unique"], 1
    )
    totals["cross_worker_visit_rate"] = totals["cross_worker_visits"] / max(
        totals["visits"], 1
    )
    totals["within_worker_revisit_rate"] = totals["within_worker_revisits"] / max(
        totals["visits"], 1
    )
    totals["drop_rate"] = totals["dropped"] / max(totals["visits"], 1)
    known_hits = totals["tt_local_hits"] + totals["tt_foreign_hits"]
    totals["foreign_tt_hit_rate"] = totals["tt_foreign_hits"] / max(known_hits, 1)
    totals["tt_unknown_hit_rate"] = totals["tt_unknown_hits"] / max(
        totals["tt_sampled_hits"], 1
    )
    return totals


def summarize_lazy_root_work(rows: list[dict]) -> dict[str, float | int] | None:
    """Pool selected and discarded root work across diagnostic searches."""
    diagnostics = [row.get("lazy_smp") for row in rows]
    diagnostics = [item for item in diagnostics if item is not None]
    if not diagnostics:
        return None
    totals: dict[str, float | int] = {
        "total_nodes": sum(row["nodes"] for row in rows if row.get("lazy_smp")),
        "discarded_root_nodes": sum(
            item["discarded_root_nodes"] for item in diagnostics
        ),
        "divergent_discarded_nodes": sum(
            item["divergent_discarded_nodes"] for item in diagnostics
        ),
    }
    total_nodes = int(totals["total_nodes"])
    discarded = int(totals["discarded_root_nodes"])
    totals["selected_root_nodes"] = max(total_nodes - discarded, 0)
    totals["discarded_root_node_rate"] = discarded / max(total_nodes, 1)
    totals["divergent_discarded_node_rate"] = int(
        totals["divergent_discarded_nodes"]
    ) / max(total_nodes, 1)
    return totals


def summarize_selected_tt_reuse(rows: list[dict]) -> dict[str, float | int] | None:
    """Pool sampled TT origins for workers whose root result was selected."""
    diagnostics = [row.get("lazy_smp") for row in rows]
    diagnostics = [
        item
        for item in diagnostics
        if item is not None and "selected_tt_foreign_hits" in item
    ]
    if not diagnostics:
        return None
    totals: dict[str, float | int] = {
        key: sum(item[key] for item in diagnostics)
        for key in (
            "selected_tt_local_hits",
            "selected_tt_foreign_hits",
            "selected_tt_unknown_hits",
        )
    }
    known = int(totals["selected_tt_local_hits"]) + int(
        totals["selected_tt_foreign_hits"]
    )
    totals["selected_foreign_tt_hit_rate"] = int(
        totals["selected_tt_foreign_hits"]
    ) / max(known, 1)
    sampled = known + int(totals["selected_tt_unknown_hits"])
    totals["selected_tt_unknown_hit_rate"] = int(
        totals["selected_tt_unknown_hits"]
    ) / max(sampled, 1)
    return totals


def summarize_lazy_root_hints(
    rows: list[dict], *, require_all: bool = False
) -> dict[str, float | int | None] | None:
    """Pool root-hint opportunities and actual ordering changes."""
    diagnostics = [row.get("lazy_smp_root_hint") for row in rows]
    if require_all and any(item is None for item in diagnostics):
        raise ValueError("root-hint summary contains a missing diagnostic")
    diagnostics = [item for item in diagnostics if item is not None]
    if not diagnostics:
        return None
    totals: dict[str, float | int] = {
        key: sum(item[key] for item in diagnostics)
        for key in ("publications", "eligible_reads", "applied")
    }
    transfer = [item for item in diagnostics if "first_move_beta_cuts" in item]
    if transfer:
        if len(transfer) != len(diagnostics):
            raise ValueError("root-hint summary mixes legacy and transfer diagnostic schemas")
        for key in ROOT_HINT_TRANSFER_KEYS[3:]:
            totals[key] = (max(item[key] for item in transfer) if key.endswith("_max")
                           else sum(item[key] for item in transfer))
        totals["first_move_beta_cut_rate"] = (
            totals["first_move_beta_cuts"] / totals["applied"] if totals["applied"] else None
        )
        totals["mean_depth_gap"] = (
            totals["depth_gap_sum"] / totals["eligible_reads"] if totals["eligible_reads"] else None
        )
    totals["searches"] = len(diagnostics)
    totals["searches_with_publication"] = sum(
        item["publications"] > 0 for item in diagnostics
    )
    totals["searches_with_application"] = sum(item["applied"] > 0 for item in diagnostics)
    totals["publication_search_rate"] = int(
        totals["searches_with_publication"]
    ) / len(diagnostics)
    totals["application_search_rate"] = int(
        totals["searches_with_application"]
    ) / len(diagnostics)
    totals["reads_per_publication"] = (
        int(totals["eligible_reads"]) / int(totals["publications"])
        if totals["publications"]
        else None
    )
    totals["application_rate"] = (
        int(totals["applied"]) / int(totals["eligible_reads"])
        if totals["eligible_reads"]
        else None
    )
    return totals


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def effective_positions_sha256(positions: list[str]) -> str:
    canonical = "".join(f"{position}\n" for position in positions).encode("utf-8")
    return hashlib.sha256(canonical).hexdigest()


def option_value(options: list[str], name: str) -> str | None:
    value = None
    for option in options:
        option_name, separator, option_value_text = option.partition("=")
        if separator and option_name == name:
            value = option_value_text
    return value


def format_optional_ratio(value: object, *, percentage: bool) -> str:
    if value is None:
        return "n/a"
    number = float(value)
    return f"{number:.1%}" if percentage else f"{number:.2f}"


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
        if args.lazy_diagnostics or getattr(args, "require_lazy_diagnostics", False):
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
    engine = Usi(args.engine, env, timeout=getattr(args, "timeout", 120))
    rows = []
    try:
        engine.send("usi")
        handshake = engine.wait_for("usiok")
        advertised = {line[len("option name "):].split(" type ", 1)[0]
                      for line in handshake if line.startswith("option name ")}
        effective_options = engine_options(args, threads)
        # Sekirei builds without opening-book cannot enable a book and do not
        # advertise UseBook. Record the absent feature, rather than sending an
        # unsupported false setting or rejecting a valid no-book build.
        if not args.yaneuraou and "UseBook" not in advertised:
            effective_options = [option for option in effective_options if option != "UseBook=false"]
        if getattr(args, "strict_options", False):
            missing = [option.partition("=")[0] for option in effective_options
                       if option.partition("=")[0] not in advertised]
            if missing:
                raise ValueError("engine does not advertise requested options: " + ", ".join(missing))
        for option in effective_options:
            name, _, value = option.partition("=")
            engine.send(f"setoption name {name} value {value}")
        engine.send("isready")
        engine.wait_for("readyok")
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
                lazy_smp_overlap = parse_lazy_smp_overlap(lines)
                if args.require_lazy_diagnostics:
                    require_lazy_diagnostic_record(
                        lazy_smp, lazy_smp_tt, lazy_smp_overlap
                    )
                lazy_smp_root_hint = parse_lazy_smp_root_hint(
                    lines, required=args.require_root_hint_diagnostics
                )
                if nps == 0 and elapsed > 0:
                    nps = int(nodes / elapsed)
                row = {
                    "schema": "sekirei.lazy-smp-scaling-row.v2",
                    "threads": threads,
                    "position": index,
                    "repeat": repeat,
                    "bestmove": parse_bestmove(lines),
                    **parse_search_result(lines),
                    "retirement": parse_retirement(lines),
                    "nmp": parse_nmp(lines),
                    "cooperative_root": parse_cooperative_root(lines, total_nodes=nodes),
                    "depth": depth,
                    "nodes": nodes,
                    "nps": nps,
                    "elapsed_ms": round(elapsed * 1000, 3),
                    "wall_nps": nodes / elapsed if elapsed > 0 else None,
                    "provenance": {
                        **args.provenance,
                        "options": effective_options,
                        "opening_book_available": "UseBook" in advertised if not args.yaneuraou else None,
                        "budget": go_command(args),
                    },
                }
                if lazy_smp is not None:
                    row["lazy_smp"] = lazy_smp
                if lazy_smp_tt is not None:
                    row["lazy_smp_tt"] = lazy_smp_tt
                if lazy_smp_overlap is not None:
                    row["lazy_smp_overlap"] = lazy_smp_overlap
                if lazy_smp_root_hint is not None:
                    row["lazy_smp_root_hint"] = lazy_smp_root_hint
                rows.append(row)
                lazy_suffix = ""
                if lazy_smp is not None:
                    lazy_suffix = (
                        f" selected w{lazy_smp['selected_worker']}"
                        f" share {lazy_smp['selected_node_share_permille'] / 10:.1f}%"
                        f" agree {lazy_smp['move_agreement']}/{lazy_smp['workers']}"
                        f" lag {lazy_smp['stop_lag_ms']}ms"
                    )
                    if "selected_tt_foreign_hits" in lazy_smp:
                        selected_known_hits = (
                            lazy_smp["selected_tt_local_hits"]
                            + lazy_smp["selected_tt_foreign_hits"]
                        )
                        lazy_suffix += (
                            f" selected-foreign-tt"
                            f" {lazy_smp['selected_tt_foreign_hits']}"
                            f"/{max(selected_known_hits, 1)}"
                        )
                if lazy_smp_tt is not None:
                    hit_rate = lazy_smp_tt["hits"] / max(lazy_smp_tt["probes"], 1)
                    lazy_suffix += (
                        f" tt-hit {hit_rate:.1%}"
                        f" collisions {lazy_smp_tt['collision_overwrites']}"
                    )
                if lazy_smp_overlap is not None:
                    cross_rate = lazy_smp_overlap["cross_worker_visits"] / max(
                        lazy_smp_overlap["visits"], 1
                    )
                    shared_rate = lazy_smp_overlap["shared_fingerprints"] / max(
                        lazy_smp_overlap["unique"], 1
                    )
                    lazy_suffix += (
                        f" cross-work {cross_rate:.1%}"
                        f" shared-fp {shared_rate:.1%}"
                        f" foreign-tt {lazy_smp_overlap['tt_foreign_hits']}"
                        f"/{max(lazy_smp_overlap['tt_local_hits'] + lazy_smp_overlap['tt_foreign_hits'], 1)}"
                    )
                if lazy_smp_root_hint is not None:
                    lazy_suffix += (
                        f" root-hint pub {lazy_smp_root_hint['publications']}"
                        f" read {lazy_smp_root_hint['eligible_reads']}"
                        f" applied {lazy_smp_root_hint['applied']}"
                    )
                print(
                    f"  {getattr(args, 'label', '')}threads {threads} pos {index + getattr(args, 'position_offset', 0):2d} run {repeat + 1}:"
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
    parser.add_argument("--timeout", type=float, default=120, help="maximum seconds per USI response")
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
        "--require-lazy-diagnostics",
        action="store_true",
        help=(
            "fail unless every search emits complete Lazy SMP worker, TT, overlap, "
            "and selected-worker TT-origin diagnostics"
        ),
    )
    parser.add_argument(
        "--require-root-hint-diagnostics",
        action="store_true",
        help=(
            "fail unless every search emits one valid helper-root-hint record; "
            "requires Sekirei LazyFlags bit 2048 and at least two threads"
        ),
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
    if args.yaneuraou and args.require_lazy_diagnostics:
        parser.error("--require-lazy-diagnostics is Sekirei-only")
    if args.yaneuraou and args.require_root_hint_diagnostics:
        parser.error("--require-root-hint-diagnostics is Sekirei-only")
    if args.timeout <= 0:
        parser.error("--timeout must be positive")
    if args.repeats < 1:
        parser.error("--repeats must be at least 1")
    if args.depth is not None and args.depth < 1:
        parser.error("--depth must be at least 1")

    positions = read_positions(args.positions, args.count)
    thread_counts = [int(t) for t in args.threads.split(",")]
    if args.require_root_hint_diagnostics:
        lazy_flags = option_value(args.option, "LazyFlags")
        try:
            root_hint_enabled = (
                lazy_flags is not None and int(lazy_flags) & ROOT_HINT_FLAG != 0
            )
        except ValueError:
            root_hint_enabled = False
        if not root_hint_enabled:
            parser.error(
                "--require-root-hint-diagnostics requires --option LazyFlags=<value> "
                "with bit 2048 set"
            )
        if any(threads < 2 for threads in thread_counts):
            parser.error(
                "--require-root-hint-diagnostics requires every thread count to be at least 2"
            )
    engine_path = Path(args.engine).resolve()
    positions_path = Path(args.positions).resolve()
    args.provenance = {
        "engine": str(engine_path),
        "engine_sha256": sha256_file(engine_path),
        "positions": str(positions_path),
        "positions_file_sha256": sha256_file(positions_path),
        "effective_positions_sha256": effective_positions_sha256(positions),
        "effective_position_count": len(positions),
        "evaluator": (
            {
                "kind": "halfkp",
                "path": str(Path(args.evalfile).resolve()),
                "sha256": sha256_file(Path(args.evalfile).resolve()),
            }
            if args.evalfile is not None
            else {"kind": "material-only", "path": None, "sha256": None}
        ),
        "host": {
            "system": platform.system(),
            "release": platform.release(),
            "machine": platform.machine(),
            "processor": platform.processor(),
        },
        "started_at_utc": datetime.now(timezone.utc).isoformat(),
        "counter_overhead_included": bool(
            args.lazy_diagnostics
            or args.require_lazy_diagnostics
            or args.require_root_hint_diagnostics
        ),
    }
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
    overlap_summaries = {
        threads: summarize_lazy_overlap(results[threads]) for threads in thread_counts
    }
    if any(summary is not None for summary in overlap_summaries.values()):
        print("Sampled work-overlap lane (1/sample-rate positions; diagnostic only)")
        print("threads  cross-worker visits  shared fingerprints  within-worker revisits  dropped")
        for threads in thread_counts:
            summary = overlap_summaries[threads]
            if summary is None:
                continue
            print(
                f"{threads:7d}  {summary['cross_worker_visit_rate']:19.1%}"
                f"  {summary['shared_fingerprint_rate']:19.1%}"
                f"  {summary['within_worker_revisit_rate']:22.1%}"
                f"  {summary['drop_rate']:7.3%}"
            )
        print("Sampled TT reuse (known writer only; diagnostic only)")
        print("threads  foreign-worker hit  unknown writer  dropped stores")
        for threads in thread_counts:
            summary = overlap_summaries[threads]
            if summary is None:
                continue
            print(
                f"{threads:7d}  {summary['foreign_tt_hit_rate']:18.1%}"
                f"  {summary['tt_unknown_hit_rate']:14.1%}"
                f"  {summary['tt_dropped_stores']:14d}"
            )
    root_work_summaries = {
        threads: summarize_lazy_root_work(results[threads]) for threads in thread_counts
    }
    if any(summary is not None for summary in root_work_summaries.values()):
        print("Root-result disposition (all workers; diagnostic only)")
        print("threads  discarded root work  divergent discarded work")
        for threads in thread_counts:
            summary = root_work_summaries[threads]
            if summary is None:
                continue
            print(
                f"{threads:7d}  {summary['discarded_root_node_rate']:19.1%}"
                f"  {summary['divergent_discarded_node_rate']:25.1%}"
            )
    selected_reuse_summaries = {
        threads: summarize_selected_tt_reuse(results[threads])
        for threads in thread_counts
    }
    if any(summary is not None for summary in selected_reuse_summaries.values()):
        print("Selected-worker sampled TT reuse (diagnostic only)")
        print("threads  foreign-worker hit  unknown writer")
        for threads in thread_counts:
            summary = selected_reuse_summaries[threads]
            if summary is None:
                continue
            print(
                f"{threads:7d}  {summary['selected_foreign_tt_hit_rate']:18.1%}"
                f"  {summary['selected_tt_unknown_hit_rate']:14.1%}"
            )
    root_hint_summaries = {
        threads: summarize_lazy_root_hints(
            results[threads], require_all=args.require_root_hint_diagnostics
        )
        for threads in thread_counts
    }
    if any(summary is not None for summary in root_hint_summaries.values()):
        print("Helper-root-hint transfer (diagnostic only)")
        print("threads  accepted/search  apply/search  depth-reads/accepted  applied/read")
        for threads in thread_counts:
            summary = root_hint_summaries[threads]
            if summary is None:
                continue
            print(
                f"{threads:7d}  {summary['publication_search_rate']:14.1%}"
                f"  {summary['application_search_rate']:12.1%}"
                f"  {format_optional_ratio(summary['reads_per_publication'], percentage=False):>17}"
                f"  {format_optional_ratio(summary['application_rate'], percentage=True):>12}"
            )
    return 0


if __name__ == "__main__":
    sys.exit(main())
