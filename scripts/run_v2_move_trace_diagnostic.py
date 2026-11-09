#!/usr/bin/env python3
"""Trace legal replies after one root move with a bounded, single-thread batch.

The counters cover repeated visits across iterative deepening and are diagnostic
only. A traced reply is not a best-move oracle or a playing-strength result.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import time
from pathlib import Path
from typing import Callable

from run_fixed_selfplay_diagnostic import parse_fields
from run_tactical_line_diagnostic import atomic_json, run_process, sha256


TRACE_KEYS = (
    "trace_pre_tt_nodes", "trace_move_loops", "trace_generated",
    "trace_considered", "trace_lmp", "trace_quiet_history",
    "trace_quiet_futility", "trace_quiet_see", "trace_tactical_futility",
    "trace_tactical_see", "trace_searched", "trace_reduced",
    "trace_excluded", "trace_quiet_stage_skipped",
    "trace_filtered_nonpromotion",
)
IDENTITY_KEYS = (
    "bestmove", "score_cp", "depth", "nodes", "completed_bound", "pv_usi"
)


def parse_result(fields: dict[str, str], reply: str | None) -> dict:
    if fields["completed_iteration_valid"] != "true" or fields["completed_bound"] != "exact":
        raise ValueError(f"{reply or 'baseline'}: no completed exact iteration")
    if (fields["pv_legal"] != "true" or fields["pv_replay_preserves_input"] != "true"
            or fields.get("search_board_restored") != "true"):
        raise ValueError(f"{reply or 'baseline'}: illegal PV or mutated input")
    result = {
        "bestmove": fields["bestmove"],
        "score_cp": int(fields["score_cp"]),
        "depth": int(fields["depth"]),
        "nodes": int(fields["nodes"]),
        "completed_bound": fields["completed_bound"],
        "pv_usi": fields["pv_usi"],
    }
    if reply is not None:
        if fields["trace_target_move"] != reply:
            raise ValueError(f"trace target differs from requested reply: {reply}")
        result["target_hash"] = fields["trace_target_hash"]
        result["trace"] = {key: int(fields[key]) for key in TRACE_KEYS}
    return result


def run_trace_batch(search: Callable[[str | None], dict], replies: list[str]) -> tuple[dict | None, list[dict], dict | None]:
    baseline = None
    rows = []
    for reply in [None, *replies]:
        try:
            observed = search(reply)
            if reply is None:
                baseline = observed
                continue
            assert baseline is not None
            rows.append({
                "reply": reply,
                "target_hash": observed["target_hash"],
                "trace": observed["trace"],
                "identity_matches_baseline": all(observed[key] == baseline[key]
                                                 for key in IDENTITY_KEYS),
            })
            if not rows[-1]["identity_matches_baseline"]:
                raise ValueError("trace changed the fixed-root search result")
        except (subprocess.TimeoutExpired, TimeoutError, RuntimeError, ValueError, KeyError,
                OSError) as error:
            return baseline, rows, {"reply": reply, "reason": type(error).__name__,
                                    "detail": str(error)[:500]}
    return baseline, rows, None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--sfen", required=True)
    parser.add_argument("--root-move", required=True)
    parser.add_argument("--reply", action="append", required=True)
    parser.add_argument("--nodes", type=int, required=True)
    parser.add_argument("--max-invocations", type=int, required=True)
    parser.add_argument("--max-total-seconds", type=int, default=90)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if not args.engine.is_file():
        parser.error(f"engine does not exist: {args.engine}")
    if len(args.sfen.split()) != 4:
        parser.error("--sfen must have four fields")
    if not 1 <= args.nodes <= 50000 or not 1 <= args.max_total_seconds <= 90 \
            or not 1 <= args.max_invocations <= 24:
        parser.error("nodes and limits must fit 50000 nodes, 24 invocations, 90 seconds")
    if len(args.reply) != len(set(args.reply)):
        parser.error("--reply moves must be unique")
    if 1 + len(args.reply) > args.max_invocations:
        parser.error(f"batch requires {1 + len(args.reply)} invocations")

    deadline = time.monotonic() + args.max_total_seconds

    def search(reply: str | None) -> dict:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise TimeoutError("batch deadline exhausted before another search")
        command = [str(args.engine), "--nodes", str(args.nodes), "--sfen", args.sfen,
                   "--root-move", args.root_move]
        if reply is not None:
            command.extend(["--trace-after-root-move", args.root_move,
                            "--trace-move", reply])
        observed = parse_result(parse_fields(run_process(command, timeout=remaining).stdout), reply)
        if observed["bestmove"] != args.root_move or not observed["pv_usi"].split(",")[0] == args.root_move:
            raise ValueError("fixed-root search returned a different root or PV")
        return observed

    baseline, rows, failure = run_trace_batch(search, args.reply)
    status = "complete" if failure is None else "interrupted"
    report = {
        "schema": "sekirei.v2-targeted-move-trace.v1",
        "diagnostic_only": True,
        "strength_claim": False,
        "status": status,
        "failure": failure,
        "planned_invocations": 1 + len(args.reply),
        "completed_invocations": int(baseline is not None) + len(rows),
        "conditions": {
            "threads": 1,
            "evaluator": "material",
            "nodes_per_fresh_process": args.nodes,
            "invocations": 1 + len(args.reply),
            "engine": str(args.engine),
            "engine_sha256": sha256(args.engine),
            "sfen": args.sfen,
            "root_move": args.root_move,
            "max_total_seconds": args.max_total_seconds,
        },
        "baseline": baseline,
        "rows": rows,
    }
    atomic_json(args.output, report)
    print(json.dumps({"status": status, "rows": len(rows), "output": str(args.output),
                      "all_identity": all(row["identity_matches_baseline"] for row in rows)},
                     sort_keys=True))
    return 0 if status == "complete" else 1


if __name__ == "__main__":
    raise SystemExit(main())
