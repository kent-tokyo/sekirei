#!/usr/bin/env python3
"""Profile one fixed search corpus without turning a cost result into Elo evidence."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import statistics
import subprocess
from pathlib import Path
from typing import Any


SCHEMA = "sekirei.q26-search-profile.v3"
RESULT_KEYS = (
    "bestmove",
    "depth",
    "score_cp",
    "nodes",
    "bound",
    "completed_bound",
    "completed_iteration_valid",
    "aborted",
    "abort_reason",
    "pv_usi",
    "pv_legal",
    "pv_replay_preserves_input",
    "history_matches_expected",
)
LEAF_COMPONENTS = (
    "static_evaluation_ns",
    "tt_probe_ns",
    "tt_store_ns",
    "movegen_generate_ns",
    "move_order_ns",
    "move_order_score_ns",
    "move_order_sort_ns",
    "root_mate_safety_ns",
)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def parse(line: str) -> dict[str, Any]:
    values = dict(field.split("=", 1) for field in line.strip().split("\t") if "=" in field)
    for key in (
        "depth", "score_cp", "nodes", "elapsed_ms", "elapsed_ns", "static_evaluations",
        "eval_cache_probes", "eval_cache_hits", "tt_probes", "tt_hits", "tt_stores", "order_tt",
        "order_killer", "order_countermove", "order_history", "root_mate_in_one_nodes",
        "root_mate_blunder_nodes", "root_mate_in_one_cache_hits",
        "root_mate_blunder_cache_hits", "alpha_beta_calls", "quiescence_calls",
        "static_evaluation_ns", "tt_probe_ns", "tt_store_ns", "movegen_order_ns",
        "movegen_generate_ns", "move_order_ns", "move_order_score_ns", "move_order_sort_ns",
        "quiescence_inclusive_ns", "root_mate_safety_ns",
    ):
        values[key] = int(values[key])
    for key in ("completed_iteration_valid", "aborted", "pv_legal", "pv_replay_preserves_input", "history_matches_expected"):
        values[key] = values[key] == "true"
    return values


def run(binary: Path, position: dict[str, Any], nodes: int, time_ms: int | None, max_depth: int, weights: Path | None, profile: bool) -> dict[str, Any]:
    # Q21s records a game-history fragment for analysis, but that fragment is
    # not replayable from every saved initial_sfen. Q26 profiles the recorded
    # current position only; a broken history must not silently turn this into
    # a different position or an invalid timing record.
    command = [str(binary), "--max-depth", str(max_depth), "--sfen", position["sfen"]]
    command.extend(("--time-ms", str(time_ms)) if time_ms is not None else ("--nodes", str(nodes)))
    command.extend(("--expected-sfen", position["sfen"]))
    if profile:
        command.append("--profile-cost")
    if weights is not None:
        command.extend(("--weights", str(weights), "--nnue-output", "residual-material"))
    env = dict(os.environ, RAYON_NUM_THREADS="1")
    completed = subprocess.run(command, text=True, capture_output=True, env=env, timeout=90, check=False)
    if completed.returncode:
        raise RuntimeError(f"search failed: {completed.stderr[-1000:]}")
    return parse(completed.stdout)


def same_result(left: dict[str, Any], right: dict[str, Any]) -> bool:
    return all(left[key] == right[key] for key in RESULT_KEYS)


def structurally_valid(result: dict[str, Any]) -> bool:
    return result["pv_legal"] and result["pv_replay_preserves_input"] and result["history_matches_expected"]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--weights", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--nodes", type=int, default=100_000)
    parser.add_argument("--time-ms", type=int)
    parser.add_argument("--positions", type=int, default=8)
    parser.add_argument("--max-depth", type=int, default=4)
    args = parser.parse_args()
    if args.nodes <= 0 or args.positions <= 0 or args.max_depth <= 0 or (args.time_ms is not None and args.time_ms <= 0):
        parser.error("nodes, positions, and max-depth must be positive")
    for path in (args.corpus, args.binary, args.weights):
        if not path.is_file():
            parser.error(f"missing input: {path}")
    corpus = json.loads(args.corpus.read_text(encoding="utf-8"))
    positions = corpus.get("positions", [])[: args.positions]
    if len(positions) != args.positions or not all("position" in item for item in positions):
        parser.error("corpus does not contain the requested number of replayable positions")

    args.output_dir.mkdir(parents=True, exist_ok=True)
    prereg = {
        "schema": SCHEMA,
        "status": "frozen_before_measurement",
        "diagnostic_only": True,
        "strength_claim": False,
        "contract": {
            "arms": {"material": "no weights", "frozen_nnue": "residual-material, scale=1000"},
            "nodes": args.nodes,
            "time_ms": args.time_ms,
            "max_depth": args.max_depth,
            "threads": 1,
            "runs_per_arm_position": ["baseline_a", "profile_a", "baseline_b", "profile_b"],
            "result_identity": list(RESULT_KEYS),
            "timing_model": "inclusive and non-additive; no component sum is reported as total CPU time",
            "position_replay": "current SFEN only; Q21s history fragments are intentionally excluded",
            "selection_components": list(LEAF_COMPONENTS),
            "excluded_from_strength_claim": "existing eval-cache engineering improvement",
        },
        "inputs": {
            "corpus": {"path": str(args.corpus), "sha256": sha256(args.corpus)},
            "binary": {"path": str(args.binary), "sha256": sha256(args.binary)},
            "frozen_nnue": {"path": str(args.weights), "sha256": sha256(args.weights)},
        },
        "positions": [{"id": item["id"], "sfen": item["position"]["sfen"]} for item in positions],
    }
    prereg_path = args.output_dir / "preregistration.json"
    rendered = json.dumps(prereg, indent=2, sort_keys=True) + "\n"
    if prereg_path.exists() and prereg_path.read_text(encoding="utf-8") != rendered:
        parser.error("existing preregistration differs")
    prereg_path.write_text(rendered, encoding="utf-8")

    rows: list[dict[str, Any]] = []
    for arm, weights in (("material", None), ("frozen_nnue", args.weights)):
        for item in positions:
            position = item["position"]
            results = {
                "baseline_a": run(args.binary, position, args.nodes, args.time_ms, args.max_depth, weights, False),
                "profile_a": run(args.binary, position, args.nodes, args.time_ms, args.max_depth, weights, True),
                "baseline_b": run(args.binary, position, args.nodes, args.time_ms, args.max_depth, weights, False),
                "profile_b": run(args.binary, position, args.nodes, args.time_ms, args.max_depth, weights, True),
            }
            identity_ok = all(same_result(results["baseline_a"], result) for result in results.values())
            structural_ok = all(structurally_valid(result) for result in results.values())
            rows.append({"arm": arm, "position_id": item["id"], "result_identity": identity_ok, "structural_valid": structural_ok, "results": results})

    profiles = [row["results"][name] for row in rows for name in ("profile_a", "profile_b")]
    timing = {
        arm: {
            "baseline_median_elapsed_ns": statistics.median(
                row["results"][name]["elapsed_ns"] for row in rows if row["arm"] == arm for name in ("baseline_a", "baseline_b")
            ),
            "profile_median_elapsed_ns": statistics.median(
                row["results"][name]["elapsed_ns"] for row in rows if row["arm"] == arm for name in ("profile_a", "profile_b")
            ),
        }
        for arm in ("material", "frozen_nnue")
    }
    for values in timing.values():
        values["instrumentation_elapsed_ratio"] = values["profile_median_elapsed_ns"] / values["baseline_median_elapsed_ns"]
    component_medians = {key: statistics.median(profile[key] for profile in profiles) for key in LEAF_COMPONENTS}
    tt_breakdown = {
        "probe_median_ns": component_medians["tt_probe_ns"],
        "store_median_ns": component_medians["tt_store_ns"],
        "probe_median_count": statistics.median(profile["tt_probes"] for profile in profiles),
        "hit_median_count": statistics.median(profile["tt_hits"] for profile in profiles),
        "store_median_count": statistics.median(profile["tt_stores"] for profile in profiles),
    }
    tt_breakdown["hit_rate"] = (
        tt_breakdown["hit_median_count"] / tt_breakdown["probe_median_count"]
        if tt_breakdown["probe_median_count"]
        else 0.0
    )
    tt_breakdown["probe_ns_per_call"] = (
        tt_breakdown["probe_median_ns"] / tt_breakdown["probe_median_count"]
        if tt_breakdown["probe_median_count"]
        else 0.0
    )
    tt_breakdown["store_ns_per_call"] = (
        tt_breakdown["store_median_ns"] / tt_breakdown["store_median_count"]
        if tt_breakdown["store_median_count"]
        else 0.0
    )
    ordering_breakdown = {
        "inclusive_median_ns": component_medians["move_order_ns"],
        "sort_inclusive_median_ns": component_medians["move_order_sort_ns"],
        "score_nested_median_ns": component_medians["move_order_score_ns"],
    }
    ordering_breakdown["sort_exclusive_median_ns"] = max(
        0,
        ordering_breakdown["sort_inclusive_median_ns"] - ordering_breakdown["score_nested_median_ns"],
    )
    selected = max(component_medians, key=component_medians.get)
    report = {
        "schema": SCHEMA,
        "status": "complete" if all(
            row["structural_valid"] and (args.time_ms is not None or row["result_identity"])
            for row in rows
        ) else "fail_result_contract",
        "diagnostic_only": True,
        "strength_claim": False,
        "preregistration_sha256": sha256(prereg_path),
        "rows": rows,
        "timing": timing,
        "component_median_ns": component_medians,
        "tt_breakdown": tt_breakdown,
        "ordering_breakdown": ordering_breakdown,
        "quiescence_inclusive_median_ns": statistics.median(profile["quiescence_inclusive_ns"] for profile in profiles),
        "selected_single_component": selected,
        "selection_reason": "largest median independently timed leaf component; quiescence is reported separately because it overlaps leaf spans",
    }
    (args.output_dir / "report.json").write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(f"status={report['status']} selected={selected}")
    return 0 if report["status"] == "complete" else 2


if __name__ == "__main__":
    raise SystemExit(main())
