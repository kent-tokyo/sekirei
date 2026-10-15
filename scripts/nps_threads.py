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
playing-strength claims. A readyok barrier after every position keeps pending
new-game reset work outside elapsed_ms; NPS still uses the engine's last info
value, falling back to nodes/elapsed only when that value is zero.
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
import stat
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from collections.abc import Iterable

ROOT = Path(__file__).resolve().parent.parent
ROOT_HINT_FLAG = 2048
ROOT_HINT_KEYS = ("publications", "eligible_reads", "applied")
ROOT_HINT_TRANSFER_KEYS = ROOT_HINT_KEYS + (
    "first_move_beta_cuts", "researches", "research_nodes", "stale_research_nodes", "stale_reads",
    "depth_gap_sum", "depth_gap_max", "generation_gap_sum", "generation_gap_max",
)
ELAPSED_SCOPE = "go-send-to-bestmove-after-position-readyok"


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


def parse_cooperative_root(
    lines: list[str], *, total_nodes: int | None = None, workers: int | None = None,
) -> dict[str, int] | None:
    group_prefix = "info string cooperative_root_group"
    groups = [line[len(group_prefix):].split() for line in lines
              if line == group_prefix or line.startswith(group_prefix + " ")]
    group = None
    if groups:
        fields = groups[0]
        if len(groups) != 1 or len(fields) != 4 or fields[::2] != ["workers", "helpers"]:
            raise ValueError("malformed cooperative root group diagnostic")
        if any(not value.isascii() or not value.isdigit() for value in fields[1::2]):
            raise ValueError("cooperative root group counters must be unsigned decimal integers")
        group = dict(zip(("workers", "helpers"), map(int, fields[1::2]), strict=True))
        if any(value > (1 << 32) - 1 for value in group.values()):
            raise ValueError("cooperative root group counter exceeds u32")
        if group["workers"] not in (4, 8) or group["helpers"] != group["workers"] - 1:
            raise ValueError("invalid cooperative root group size")
    prefix = "info string cooperative_root "
    matches = [line[len(prefix):] for line in lines if line.startswith(prefix)]
    if not matches:
        if group is not None:
            raise ValueError("cooperative root group requires root counters")
        return None
    actual_workers = group["workers"] if group is not None else 2
    if workers is not None and workers != actual_workers:
        raise ValueError("cooperative root workers differ from requested threads")
    helpers_per_generation = actual_workers - 1
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
            "physical_helper_threads_started",
            "physical_helper_threads_joined",
            "executor_start_delay_us",
            "helper_dispatch_delay_us",
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
    if result["spawn_failures"] + result["helpers_started"] != result["generations"] * helpers_per_generation or result["helper_aborted_probes"] > result["helpers_started"]:
        raise ValueError("cooperative lifecycle counts inconsistent")
    if result["main_confirmations"] == 0 and result["alpha_gap_sum"] != 0:
        raise ValueError("cooperative alpha gap without confirmations")
    if result["helpers_started"] == 0 and any(result[key] != 0 for key in (
        "helper_probes", "helper_nodes", "helper_start_delay_us", "join_wait_us",
        "executor_start_delay_us", "helper_dispatch_delay_us",
    )):
        raise ValueError("cooperative work without started helpers")
    if total_nodes is not None and result["main_nodes"] + result["helper_nodes"] > total_nodes:
        raise ValueError("cooperative classified nodes exceed total nodes")
    if result["physical_helper_threads_started"] != result["physical_helper_threads_joined"]:
        raise ValueError("cooperative helper thread was not joined")
    if result["physical_helper_threads_started"] > result["helpers_started"]:
        raise ValueError("cooperative physical threads exceed started jobs")
    if group is not None:
        result.update(group)
    return result


COOPERATIVE_ROOT_PRESTART_KEYS = ("passes", "first_beta_cuts", "first_aborts", "helper_nodes")
COOPERATIVE_ROOT_PRESTART_CAP_KEYS = (
    "limit", "passes", "initial_claims", "park_calls", "resumed_helpers", "cancelled_helpers",
)
COOPERATIVE_ROOT_PRESTART_DIAG_KEYS = (
    "helper_jobs_before_first_return", "helper_completed_probes_before_first_return",
    "helper_fail_highs_before_first_return", "first_main_nodes", "first_main_us",
    "first_beta_helper_nodes",
)


def _parse_prestart_counters(lines: list[str], prefix: str, keys: tuple[str, ...]) -> dict[str, int] | None:
    matches = [line[len(prefix):].split() for line in lines
               if line == prefix or line.startswith(prefix + " ")]
    if not matches:
        return None
    fields = matches[0]
    if len(matches) != 1 or len(fields) != len(keys) * 2 or fields[::2] != list(keys):
        raise ValueError("malformed cooperative prestart diagnostic")
    if any(not value.isascii() or not value.isdigit() for value in fields[1::2]):
        raise ValueError("cooperative prestart counters must be unsigned decimal integers")
    result = dict(zip(keys, map(int, fields[1::2]), strict=True))
    if any(value > (1 << 64) - 1 for value in result.values()):
        raise ValueError("cooperative prestart counter exceeds u64")
    return result


def parse_cooperative_root_prestart(lines: list[str], *, cooperative_root: dict[str, int] | None) -> dict[str, int] | None:
    result = _parse_prestart_counters(lines, "info string cooperative_root_prestart", COOPERATIVE_ROOT_PRESTART_KEYS)
    if result is None:
        return None
    if cooperative_root is None or cooperative_root.get("workers") not in (4, 8):
        raise ValueError("cooperative prestart requires a four/eight-worker root group")
    if result["passes"] > cooperative_root["generations"]:
        raise ValueError("prestart passes exceed root generations")
    if result["first_beta_cuts"] + result["first_aborts"] > result["passes"]:
        raise ValueError("prestart first-child outcomes exceed passes")
    if result["helper_nodes"] > cooperative_root["helper_nodes"]:
        raise ValueError("prestart helper nodes exceed total helper nodes")
    if result["passes"] == 0 and any(result.values()):
        raise ValueError("prestart work without eligible passes")
    return result


def parse_cooperative_root_prestart_cap(lines: list[str], *, cooperative_root: dict[str, int] | None,
                                       prestart: dict[str, int] | None) -> dict[str, int] | None:
    result = _parse_prestart_counters(lines, "info string cooperative_root_prestart_cap", COOPERATIVE_ROOT_PRESTART_CAP_KEYS)
    if result is None:
        return None
    if cooperative_root is None or cooperative_root.get("workers") not in (4, 8) or prestart is None:
        raise ValueError("prestart cap requires intrinsic prestart and a four/eight-worker root group")
    if not 1 <= result["limit"] <= 8:
        raise ValueError("prestart cap limit must be in 1..8")
    if result["passes"] != prestart["passes"]:
        raise ValueError("prestart cap passes differ from intrinsic prestart passes")
    if result["initial_claims"] > result["limit"] * result["passes"]:
        raise ValueError("prestart initial claims exceed capped passes")
    completed = result["resumed_helpers"] + result["cancelled_helpers"]
    if completed > cooperative_root["helpers_started"]:
        raise ValueError("prestart cap helper outcomes exceed started jobs")
    if result["park_calls"] < completed:
        raise ValueError("prestart cap helper outcomes exceed park calls")
    if result["passes"] == 0 and any(result[key] for key in COOPERATIVE_ROOT_PRESTART_CAP_KEYS[2:]):
        raise ValueError("prestart cap work without eligible passes")
    return result


def parse_cooperative_root_prestart_diag(lines: list[str], *, cooperative_root: dict[str, int] | None,
                                        prestart: dict[str, int] | None) -> dict[str, int] | None:
    result = _parse_prestart_counters(lines, "info string cooperative_root_prestart_diag", COOPERATIVE_ROOT_PRESTART_DIAG_KEYS)
    if result is None:
        return None
    if cooperative_root is None or prestart is None:
        raise ValueError("prestart observer requires intrinsic prestart evidence")
    if result["helper_jobs_before_first_return"] > min(
        cooperative_root["helpers_started"], prestart["passes"] * cooperative_root["helpers"],
    ):
        raise ValueError("prestart early jobs exceed reserved/started jobs")
    if result["helper_completed_probes_before_first_return"] > cooperative_root["helper_probes"]:
        raise ValueError("prestart early probes exceed full-depth probes")
    if result["helper_fail_highs_before_first_return"] > min(
        result["helper_completed_probes_before_first_return"], cooperative_root["helper_fail_highs"],
    ):
        raise ValueError("prestart early fail highs exceed completed probes")
    if result["helper_completed_probes_before_first_return"] and not result["helper_jobs_before_first_return"]:
        raise ValueError("prestart early probes without early jobs")
    if result["first_main_nodes"] > cooperative_root["main_nodes"]:
        raise ValueError("prestart first-child nodes exceed private main nodes")
    if result["first_beta_helper_nodes"] > prestart["helper_nodes"]:
        raise ValueError("prestart first-beta work exceeds prestart helper nodes")
    if not prestart["first_beta_cuts"] and result["first_beta_helper_nodes"]:
        raise ValueError("prestart first-beta work without first beta")
    if prestart["passes"] == 0 and any(result.values()):
        raise ValueError("prestart observations without eligible passes")
    return result


def parse_cooperative_root_wait(lines: list[str], *, cooperative_root: dict[str, int] | None) -> dict[str, int] | None:
    """Additive wait-drain evidence; absent for established and fallback paths."""
    prefix = "info string cooperative_root_wait "
    matches = [line[len(prefix):] for line in lines if line.startswith(prefix)]
    if not matches:
        return None
    keys = ("enabled", "waiting_confirmations", "waiting_confirmation_nodes",
            "waiting_alpha_raises", "notification_wait_us")
    fields = matches[0].split()
    if len(matches) != 1 or len(fields) != len(keys) * 2 or fields[::2] != list(keys):
        raise ValueError("malformed cooperative root wait diagnostic")
    if any(not value.isascii() or not value.isdigit() for value in fields[1::2]):
        raise ValueError("cooperative root wait counters must be unsigned decimal integers")
    result = dict(zip(keys, map(int, fields[1::2]), strict=True))
    if any(value > (1 << 64) - 1 for value in result.values()):
        raise ValueError("cooperative root wait counter exceeds u64")
    if result["enabled"] != 1 or cooperative_root is None:
        raise ValueError("cooperative root wait requires enabled split evidence")
    if result["waiting_confirmations"] > cooperative_root["main_confirmations"]:
        raise ValueError("waiting confirmations exceed main confirmations")
    if result["waiting_confirmation_nodes"] > cooperative_root["confirmation_nodes"]:
        raise ValueError("waiting nodes exceed confirmation nodes")
    if result["waiting_alpha_raises"] > min(result["waiting_confirmations"], cooperative_root["confirmed_alpha_raises"]):
        raise ValueError("waiting raises exceed confirmations or confirmed raises")
    return result


def parse_cooperative_root_policy(lines: list[str], *, cooperative_root: dict[str, int] | None) -> dict[str, int] | None:
    """Intrinsic opt-in policy identity; available without latency observers."""
    prefix = "info string cooperative_root_policy "
    matches = [line[len(prefix):] for line in lines if line.startswith(prefix)]
    if not matches:
        return None
    if len(matches) != 1 or matches[0].split() != ["priority", "1"]:
        raise ValueError("malformed cooperative root policy diagnostic")
    if cooperative_root is None:
        raise ValueError("cooperative root policy requires split evidence")
    return {"priority": 1}


def parse_cooperative_root_helper_lmr_policy(lines: list[str], *, cooperative_root: dict[str, int] | None) -> dict[str, int] | None:
    """Intrinsic helper-only LMR activation, independent of observer overhead."""
    prefix = "info string cooperative_root_helper_lmr_policy "
    matches = [line[len(prefix):] for line in lines if line.startswith(prefix)]
    if not matches:
        return None
    if len(matches) != 1 or matches[0].split() != ["enabled", "1"]:
        raise ValueError("malformed cooperative root helper LMR policy diagnostic")
    if cooperative_root is None:
        raise ValueError("cooperative root helper LMR policy requires split evidence")
    return {"enabled": 1}


COOPERATIVE_ROOT_HELPER_LMR_KEYS = (
    "reduced_probes", "reduced_fail_highs", "full_depth_rechecks",
    "full_depth_fail_highs", "completed_reduced_fail_lows",
)


def parse_cooperative_root_helper_lmr(lines: list[str], *, cooperative_root: dict[str, int] | None,
                                     policy: dict[str, int] | None,
                                     latency: dict[str, int] | None) -> dict[str, int] | None:
    """Observer counts; legacy helper_probes continues to mean full-depth completions."""
    prefix = "info string cooperative_root_helper_lmr "
    matches = [line[len(prefix):] for line in lines if line.startswith(prefix)]
    if not matches:
        return None
    fields = matches[0].split()
    keys = COOPERATIVE_ROOT_HELPER_LMR_KEYS
    if len(matches) != 1 or len(fields) != len(keys) * 2 or fields[::2] != list(keys):
        raise ValueError("malformed cooperative root helper LMR diagnostic")
    if any(not value.isascii() or not value.isdigit() for value in fields[1::2]):
        raise ValueError("cooperative root helper LMR counters must be unsigned decimal integers")
    result = dict(zip(keys, map(int, fields[1::2]), strict=True))
    if any(value > (1 << 64) - 1 for value in result.values()):
        raise ValueError("cooperative root helper LMR counter exceeds u64")
    if cooperative_root is None or policy is None or latency is None:
        raise ValueError("cooperative root helper LMR requires policy, latency and split evidence")
    if result["full_depth_rechecks"] != result["reduced_fail_highs"]:
        raise ValueError("cooperative helper LMR fail highs differ from recheck starts")
    if result["reduced_fail_highs"] + result["completed_reduced_fail_lows"] > result["reduced_probes"]:
        raise ValueError("cooperative helper LMR completed reductions exceed reduced probes")
    if result["full_depth_fail_highs"] > result["full_depth_rechecks"]:
        raise ValueError("cooperative helper LMR full-depth fail highs exceed rechecks")
    # Cancellation can intervene before legacy publication accounting. Do not
    # compare these completed helper rechecks to legacy helper_fail_highs.
    return result


COOPERATIVE_ROOT_LATENCY_KEYS = (
    "publications",
    "scout_publications",
    "research_publications",
    "confirmation_publications",
    "idle_publications",
    "phase_transition_publications",
    "beta_scout_publications",
    "confirmations",
    "confirmation_latency_us_sum",
    "confirmation_latency_us_max",
    "confirmation_main_nodes_sum",
    "confirmation_main_nodes_max",
    "scout_confirmations",
    "scout_delay_nodes_sum",
    "beta_scout_confirmations",
    "beta_scout_delay_nodes_sum",
    "beta_scout_confirmed_cuts",
    "unconfirmed_publications",
    "rejected_publications",
)


def parse_cooperative_root_latency(lines: list[str], *, cooperative_root: dict[str, int] | None) -> dict[str, int] | None:
    """Optional observer evidence; delay sums overlap and are not wasted nodes."""
    prefix = "info string cooperative_root_latency "
    matches = [line[len(prefix):] for line in lines if line.startswith(prefix)]
    if not matches:
        return None
    keys = ("enabled", *COOPERATIVE_ROOT_LATENCY_KEYS)
    fields = matches[0].split()
    if len(matches) != 1 or len(fields) != len(keys) * 2 or fields[::2] != list(keys):
        raise ValueError("malformed cooperative root latency diagnostic")
    if any(not value.isascii() or not value.isdigit() for value in fields[1::2]):
        raise ValueError("cooperative root latency counters must be unsigned decimal integers")
    result = dict(zip(keys, map(int, fields[1::2]), strict=True))
    if any(value > (1 << 64) - 1 for value in result.values()):
        raise ValueError("cooperative root latency counter exceeds u64")
    if result["enabled"] != 1 or cooperative_root is None:
        raise ValueError("cooperative root latency requires enabled split evidence")
    phases = ("scout_publications", "research_publications", "confirmation_publications",
              "idle_publications", "phase_transition_publications")
    if sum(result[key] for key in phases) != result["publications"]:
        raise ValueError("cooperative latency publication phases do not sum")
    if result["publications"] + result["rejected_publications"] != cooperative_root["helper_fail_highs"]:
        raise ValueError("cooperative latency publication attempts differ from helper fail highs")
    if result["confirmations"] != cooperative_root["main_confirmations"]:
        raise ValueError("cooperative latency confirmations differ from main confirmations")
    if result["confirmations"] > result["publications"] or result["unconfirmed_publications"] != result["publications"] - result["confirmations"]:
        raise ValueError("cooperative latency confirmed/unconfirmed accounting inconsistent")
    if not (result["beta_scout_publications"] <= result["scout_publications"]
            and result["scout_confirmations"] <= result["scout_publications"]
            and result["beta_scout_confirmations"] <= result["beta_scout_publications"]
            and result["beta_scout_confirmed_cuts"] <= result["beta_scout_confirmations"]
                <= result["scout_confirmations"] <= result["confirmations"]):
        raise ValueError("cooperative latency scout subsets inconsistent")
    if result["beta_scout_confirmed_cuts"] > cooperative_root["confirmed_alpha_raises"]:
        raise ValueError("cooperative latency beta cuts exceed confirmed alpha raises")
    if result["confirmation_latency_us_max"] > result["confirmation_latency_us_sum"] or result["confirmation_main_nodes_max"] > result["confirmation_main_nodes_sum"]:
        raise ValueError("cooperative latency maximum exceeds sum")
    if result["confirmation_main_nodes_max"] > cooperative_root["main_nodes"]:
        raise ValueError("cooperative latency maximum exceeds main nodes")
    if not result["beta_scout_delay_nodes_sum"] <= result["scout_delay_nodes_sum"] <= result["confirmation_main_nodes_sum"]:
        raise ValueError("cooperative latency node-delay subsets inconsistent")
    # Several published candidates can wait over the same main search interval.
    # Their summed delays must not be bounded by root/main/total node counts.
    return result


COOPERATIVE_ROOT_OUTCOME_KEYS = (
    "scout_confirmations",
    "research_confirmations",
    "confirmation_confirmations",
    "idle_confirmations",
    "phase_transition_confirmations",
    "scout_beta_cuts",
    "research_beta_cuts",
    "confirmation_beta_cuts",
    "idle_beta_cuts",
    "phase_transition_beta_cuts",
    "priority_choices",
    "priority_reorders",
    "reordered_alpha_raises",
    "reordered_beta_cuts",
    "reordered_confirmation_nodes",
)


def parse_cooperative_root_outcomes(lines: list[str], *, cooperative_root: dict[str, int] | None,
                                    latency: dict[str, int] | None,
                                    policy: dict[str, int] | None) -> dict[str, int] | None:
    """Observer outcome evidence classified by advice's publication phase."""
    prefix = "info string cooperative_root_outcomes "
    matches = [line[len(prefix):] for line in lines if line.startswith(prefix)]
    if not matches:
        return None
    keys = ("enabled", *COOPERATIVE_ROOT_OUTCOME_KEYS)
    fields = matches[0].split()
    if len(matches) != 1 or len(fields) != len(keys) * 2 or fields[::2] != list(keys):
        raise ValueError("malformed cooperative root outcomes diagnostic")
    if any(not value.isascii() or not value.isdigit() for value in fields[1::2]):
        raise ValueError("cooperative root outcomes counters must be unsigned decimal integers")
    result = dict(zip(keys, map(int, fields[1::2]), strict=True))
    if any(value > (1 << 64) - 1 for value in result.values()):
        raise ValueError("cooperative root outcomes counter exceeds u64")
    if result["enabled"] != 1 or cooperative_root is None or latency is None:
        raise ValueError("cooperative root outcomes require enabled latency and split evidence")
    phases = ("scout", "research", "confirmation", "idle", "phase_transition")
    if sum(result[f"{phase}_confirmations"] for phase in phases) != latency["confirmations"]:
        raise ValueError("cooperative outcomes confirmation phases do not sum")
    for phase in phases:
        if result[f"{phase}_beta_cuts"] > result[f"{phase}_confirmations"] or result[f"{phase}_confirmations"] > latency[f"{phase}_publications"]:
            raise ValueError("cooperative outcomes phase subsets inconsistent")
    cuts = sum(result[f"{phase}_beta_cuts"] for phase in phases)
    if cuts > cooperative_root["confirmed_alpha_raises"]:
        raise ValueError("cooperative outcomes beta cuts exceed confirmed alpha raises")
    if result["scout_confirmations"] != latency["scout_confirmations"] or latency["beta_scout_confirmed_cuts"] > result["scout_beta_cuts"]:
        raise ValueError("cooperative outcomes scout counters differ from latency evidence")
    if policy is not None:
        if result["priority_choices"] != latency["confirmations"]:
            raise ValueError("cooperative priority choices differ from confirmations")
    elif any(result[key] for key in COOPERATIVE_ROOT_OUTCOME_KEYS[-5:]):
        raise ValueError("cooperative priority counters without enabled policy")
    if not (result["reordered_beta_cuts"] <= result["reordered_alpha_raises"]
            <= result["priority_reorders"] <= result["priority_choices"]):
        raise ValueError("cooperative priority outcome subsets inconsistent")
    if result["reordered_alpha_raises"] > cooperative_root["confirmed_alpha_raises"] or result["reordered_beta_cuts"] > cuts:
        raise ValueError("cooperative reordered outcomes exceed confirmed outcomes")
    if result["reordered_confirmation_nodes"] > cooperative_root["confirmation_nodes"]:
        raise ValueError("cooperative reordered confirmation nodes exceed confirmation nodes")
    return result


ASPIRATION_KEYS = ("roots", "aspirated", "adapted", "delta_sum", "passes", "fail_low", "fail_high", "retries", "retry_nodes", "aborted")


def parse_aspiration(lines: list[str], *, total_nodes: int | None = None) -> list[dict]:
    workers = {}
    def counts(text):
        fields = text.split(",")
        if len(fields) != len(ASPIRATION_KEYS) or any(not v.isascii() or not v.isdigit() for v in fields):
            raise ValueError("malformed aspiration counts")
        values = list(map(int, fields))
        if any(v > (1 << 64) - 1 for v in values):
            raise ValueError("aspiration counter exceeds u64")
        return values
    for line in lines:
        if line.startswith("info string root_aspiration worker "):
            fields = line[len("info string root_aspiration "):].split()
            if len(fields) != 4 or fields[0] != "worker" or fields[2] != "counts" or not fields[1].isascii() or not fields[1].isdigit():
                raise ValueError("malformed aspiration summary")
            worker = int(fields[1])
            if worker in workers:
                raise ValueError("duplicate aspiration summary")
            workers[worker] = {"worker": worker, "totals": counts(fields[3]), "by_depth": [[0] * 10 for _ in range(32)], "seen": set()}
        elif line.startswith("info string root_aspiration_depth "):
            fields = line[len("info string root_aspiration_depth "):].split()
            if len(fields) != 6 or fields[::2] != ["worker", "depth", "counts"] or any(not v.isascii() or not v.isdigit() for v in (fields[1], fields[3])):
                raise ValueError("malformed aspiration depth")
            worker, depth = int(fields[1]), int(fields[3])
            if worker not in workers or depth not in range(32) or depth in workers[worker]["seen"]:
                raise ValueError("missing, invalid, or duplicate aspiration depth")
            workers[worker]["seen"].add(depth)
            workers[worker]["by_depth"][depth] = counts(fields[5])
    for record in workers.values():
        if record["totals"] != [sum(row[c] for row in record["by_depth"]) for c in range(10)]:
            raise ValueError("aspiration histogram does not match summary")
        for row in [record["totals"], *record["by_depth"]]:
            roots, aspirated, adapted, delta_sum, passes, low, high, retries, _, aborted = row
            if adapted > aspirated or aspirated > roots or passes < roots or retries != passes - roots or low + high + aborted > passes or (aspirated == 0 and delta_sum != 0):
                raise ValueError("inconsistent aspiration counters")
        del record["seen"]
    if total_nodes is not None and sum(r["totals"][8] for r in workers.values()) > total_nodes:
        raise ValueError("aspiration retry nodes exceed total nodes")
    return sorted(workers.values(), key=lambda r: r["worker"])


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


def positive_u64(value: str) -> int:
    normalized = value.lstrip("0") or "0"
    if (not value.isascii() or not value.isdigit() or len(normalized) > 20
            or not 1 <= int(normalized) <= (1 << 64) - 1):
        raise argparse.ArgumentTypeError("node limit must be a positive unsigned 64-bit decimal integer")
    return int(normalized)


def go_command(args: argparse.Namespace) -> str:
    nodes = getattr(args, "nodes", None)
    if nodes is not None:
        if getattr(args, "depth", None) is not None:
            raise ValueError("depth and node budgets are mutually exclusive")
        if isinstance(nodes, bool) or not isinstance(nodes, int) or not 1 <= nodes <= (1 << 64) - 1:
            raise ValueError("node limit must be a positive u64")
        return f"go nodes {nodes}"
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


# Filesystem binding for the native, single-file nn.bin adapter. This does
# not prove runtime architecture, loaded weights or evaluation-output identity.
# Before/after checks detect persistent drift, not transient change/restoration.
# Keep this dependency inside an already frozen driver source so independent
# corpus manifests retain their existing four-file source closure.
PROTECTED_EVALUATOR_OPTIONS = {"evalfile", "evaldir", "fv_scale", "nnueoutput"}


def reject_evaluator_overrides(options: Iterable[str]) -> None:
    for option in options:
        if not isinstance(option, str) or any(c in option for c in "\r\n\0"):
            raise ValueError("USI options must be single-line Name=Value entries")
        name, separator, value = option.partition("=")
        if not separator or not name.strip():
            raise ValueError("USI options must be Name=Value entries")
        tokens = name.split()
        if any(token.casefold() == "value" for token in tokens):
            raise ValueError("USI option names cannot contain the reserved value delimiter")
        # Preserve canonical absolute-output assertions, but never names that
        # exploit a parser reading only the first whitespace-delimited token.
        if len(tokens) == 1 and tokens[0].casefold() == "nnueoutput" and value == "absolute":
            continue
        if tokens[0].casefold() in PROTECTED_EVALUATOR_OPTIONS:
            raise ValueError("evaluator override is not allowed; use --evalfile/--fv-scale: " + name.strip())


def _lexical_absolute(path: str | Path) -> Path:
    value = str(path)
    if not value or value != value.strip() or any(c in value for c in "\r\n\0"):
        raise ValueError("evaluator/input paths must be nonempty single-line values without edge whitespace")
    candidate = Path(value).expanduser()
    # Do not resolve the final-file symlink (or normalize symlink/.. paths)
    # before choosing EvalDir: Yane opens EvalDir/nn.bin, not the target's sibling.
    return candidate if candidate.is_absolute() else Path.cwd() / candidate


def yaneuraou_eval_dir(evalfile: str | Path) -> str:
    return str(_lexical_absolute(evalfile).parent)


def file_fingerprint(path: str | Path) -> dict:
    candidate = _lexical_absolute(path)
    resolved = candidate.resolve(strict=True)
    if not resolved.is_file():
        raise ValueError("input must be a regular file: " + str(candidate))
    digest = hashlib.sha256()
    with candidate.open("rb") as source:
        before = os.fstat(source.fileno())
        if not stat.S_ISREG(before.st_mode):
            raise ValueError("input must be a regular file: " + str(candidate))
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
        after = os.fstat(source.fileno())
    fields = ("st_dev", "st_ino", "st_size", "st_mtime_ns")
    if any(getattr(before, key) != getattr(after, key) for key in fields):
        raise ValueError("input changed while hashing: " + str(candidate))
    current = candidate.resolve(strict=True)
    current_stat = current.stat()
    if current != resolved or any(getattr(current_stat, key) != getattr(after, key) for key in fields):
        raise ValueError("input path changed while hashing: " + str(candidate))
    return {"path": str(candidate), "resolved_path": str(resolved),
            "sha256": digest.hexdigest(), "size_bytes": after.st_size}


def check_file_fingerprint(fingerprint: dict) -> None:
    if file_fingerprint(fingerprint["path"]) != fingerprint:
        raise ValueError("input identity changed during the run: " + fingerprint["path"])


def bind_yaneuraou_evaluator(evalfile: str | Path, fv_scale: int,
                             option_groups: Iterable[Iterable[str]] = ()) -> dict:
    for options in option_groups:
        reject_evaluator_overrides(options)
    if type(fv_scale) is not int or not 1 <= fv_scale <= 128:
        raise ValueError("native YaneuraOu FV_SCALE must be in 1..128")
    selected = file_fingerprint(evalfile)
    eval_dir = yaneuraou_eval_dir(evalfile)
    native = file_fingerprint(Path(eval_dir) / "nn.bin")
    if (selected["sha256"], selected["size_bytes"]) != (native["sha256"], native["size_bytes"]):
        raise ValueError("requested evaluator differs from native EvalDir/nn.bin")
    return {"schema": "sekirei.yaneuraou-evaluator-binding.v1",
            "loader_contract": "native-single-file-nn.bin",
            "verification_scope": "filesystem binding, not runtime architecture or loaded-output equivalence",
            "eval_dir": eval_dir, "fv_scale": fv_scale,
            "requested": selected, "native_nnue": native}


def check_yaneuraou_evaluator(binding: dict) -> None:
    current = bind_yaneuraou_evaluator(binding["requested"]["path"], binding["fv_scale"])
    if current != binding:
        raise ValueError("YaneuraOu evaluator binding changed during the run")


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
        options = [
            f"EvalDir={yaneuraou_eval_dir(args.evalfile)}",
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
    # Side options must not override the declared evaluator or inject USI
    # commands, including on material-only and explicit Sekirei lanes.
    reject_evaluator_overrides(args.option)
    yaneuraou_binding = None
    yaneuraou_binary = None
    if args.yaneuraou:
        yaneuraou_binding = bind_yaneuraou_evaluator(
            args.evalfile, args.fv_scale, option_groups=(args.option,),
        )
        yaneuraou_binary = file_fingerprint(args.engine)
        # A CLI caller may have captured declared input identities before
        # entering this lane. Never publish that older snapshot beside a
        # contradictory, newly bound native input.
        declared_evaluator = args.provenance.get("evaluator", {})
        for declared, path_key, hash_key, fingerprint in (
            (declared_evaluator, "path", "sha256", yaneuraou_binding["requested"]),
            (args.provenance, "engine", "engine_sha256", yaneuraou_binary),
        ):
            if not isinstance(declared, dict):
                raise ValueError("declared input provenance must be an object")
            if hash_key in declared and declared[hash_key] != fingerprint["sha256"]:
                raise ValueError(f"declared {hash_key} differs from the bound input")
            if path_key in declared:
                try:
                    resolved = str(Path(declared[path_key]).expanduser().resolve(strict=True))
                except (OSError, TypeError, ValueError) as error:
                    raise ValueError(f"invalid declared {path_key} input path") from error
                if resolved != fingerprint["resolved_path"]:
                    raise ValueError(f"declared {path_key} differs from the bound input path")
    go_command(args)
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
                # Both commands may leave reset/position work queued in the
                # engine. Acknowledge it before timing the search itself.
                engine.send("isready")
                engine.wait_for("readyok")
                start = time.perf_counter()
                engine.send(go_command(args))
                lines = engine.wait_for("bestmove")
                elapsed = time.perf_counter() - start
                depth, nodes, nps = parse_info(lines)
                lazy_smp = parse_lazy_smp(lines)
                lazy_smp_tt = parse_lazy_smp_tt(lines)
                lazy_smp_overlap = parse_lazy_smp_overlap(lines)
                cooperative_root = parse_cooperative_root(lines, total_nodes=nodes, workers=threads)
                cooperative_root_prestart = parse_cooperative_root_prestart(lines, cooperative_root=cooperative_root)
                cooperative_root_prestart_cap = parse_cooperative_root_prestart_cap(
                    lines, cooperative_root=cooperative_root, prestart=cooperative_root_prestart,
                )
                cooperative_root_prestart_diag = parse_cooperative_root_prestart_diag(
                    lines, cooperative_root=cooperative_root, prestart=cooperative_root_prestart,
                )
                cooperative_root_wait = parse_cooperative_root_wait(lines, cooperative_root=cooperative_root)
                cooperative_root_latency = parse_cooperative_root_latency(lines, cooperative_root=cooperative_root)
                cooperative_root_policy = parse_cooperative_root_policy(lines, cooperative_root=cooperative_root)
                cooperative_root_helper_lmr_policy = parse_cooperative_root_helper_lmr_policy(
                    lines, cooperative_root=cooperative_root,
                )
                cooperative_root_helper_lmr = parse_cooperative_root_helper_lmr(
                    lines, cooperative_root=cooperative_root, policy=cooperative_root_helper_lmr_policy,
                    latency=cooperative_root_latency,
                )
                cooperative_root_outcomes = parse_cooperative_root_outcomes(
                    lines, cooperative_root=cooperative_root, latency=cooperative_root_latency,
                    policy=cooperative_root_policy,
                )
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
                    "aspiration": parse_aspiration(lines, total_nodes=nodes),
                    "cooperative_root": cooperative_root,
                    "cooperative_root_prestart": cooperative_root_prestart,
                    "cooperative_root_prestart_cap": cooperative_root_prestart_cap,
                    "cooperative_root_prestart_diag": cooperative_root_prestart_diag,
                    "cooperative_root_wait": cooperative_root_wait,
                    "cooperative_root_latency": cooperative_root_latency,
                    "cooperative_root_policy": cooperative_root_policy,
                    "cooperative_root_helper_lmr_policy": cooperative_root_helper_lmr_policy,
                    "cooperative_root_helper_lmr": cooperative_root_helper_lmr,
                    "cooperative_root_outcomes": cooperative_root_outcomes,
                    "depth": depth,
                    "nodes": nodes,
                    "nps": nps,
                    "elapsed_ms": round(elapsed * 1000, 3),
                    "wall_nps": nodes / elapsed if elapsed > 0 else None,
                    "provenance": {
                        **args.provenance,
                        **({"yaneuraou_evaluator_binding": yaneuraou_binding,
                            "yaneuraou_engine_fingerprint": yaneuraou_binary}
                           if yaneuraou_binding is not None else {}),
                        "elapsed_scope": ELAPSED_SCOPE,
                        "options": effective_options,
                        "cooperative_prestart_enabled": cooperative_root_prestart is not None,
                        "cooperative_prestart_cap_enabled": cooperative_root_prestart_cap is not None,
                        "cooperative_prestart_overhead_included": cooperative_root_prestart_diag is not None,
                        "cooperative_latency_overhead_included": cooperative_root_latency is not None,
                        "cooperative_priority_enabled": cooperative_root_policy is not None,
                        "cooperative_helper_lmr_enabled": cooperative_root_helper_lmr_policy is not None,
                        **({"counter_overhead_included": True, "timing_qualified": False}
                           if cooperative_root_latency is not None or cooperative_root_prestart_diag is not None else {}),
                        "opening_book_available": "UseBook" in advertised if not args.yaneuraou else None,
                        "budget": go_command(args),
                        **({"measurement_phase": "fixed_node_work_diagnostic", "node_limit": args.nodes,
                            "timing_qualified": False} if getattr(args, "nodes", None) is not None else {}),
                    },
                }
                if getattr(args, "nodes", None) is not None:
                    row["node_limit_overshoot"] = max(0, nodes - args.nodes)
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
    # Keep measured rows private until the bound inputs survive the entire
    # invocation, including shutdown. This attests filesystem identity only.
    if yaneuraou_binding is not None:
        check_yaneuraou_evaluator(yaneuraou_binding)
        check_file_fingerprint(yaneuraou_binary)
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
    budget.add_argument("--nodes", type=positive_u64, help="fixed node budget; work diagnostic without timing qualification")
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
        else (f"fixed node budget {args.nodes} (work diagnostic; timing unqualified)" if args.nodes is not None else f"{args.movetime} ms each")
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
