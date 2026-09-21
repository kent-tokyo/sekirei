#!/usr/bin/env python3
"""Measure Q21i NNUE update, forward, and search-tree cost separately.

This is a diagnostic benchmark, not strength evidence.  It freezes one
position from every Q21g phase/material stratum before launching subprocesses.
"""

import argparse
import hashlib
import json
import os
import statistics
import subprocess
import sys
from pathlib import Path


PHASES = ("opening", "middlegame", "endgame")
MATERIAL_BANDS = ("stm_behind", "balanced", "stm_ahead")
ARMS = ("material", "cost-only", "teacher")
COMPONENT_MODES = ("material", "nnue-update", "forward")
SEARCH_ORDERS = (
    ("material", "cost-only", "teacher"),
    ("teacher", "material", "cost-only"),
    ("cost-only", "teacher", "material"),
)
COMPONENT_ORDERS = (
    ("material", "nnue-update", "forward"),
    ("forward", "material", "nnue-update"),
    ("nnue-update", "forward", "material"),
)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def select_positions(entries: list[dict]) -> list[dict]:
    """Choose one score-blind position per stratum, alternating class."""
    selected = []
    for index, (phase, band) in enumerate(
        (phase, band) for phase in PHASES for band in MATERIAL_BANDS
    ):
        candidates = [
            entry
            for entry in entries
            if entry["attributes"]["phase"] == phase
            and entry["attributes"]["material_band"] == band
        ]
        wanted = "tactical" if index % 2 else "normal"
        preferred = [entry for entry in candidates if entry["selection_class"] == wanted]
        pool = preferred or candidates
        if not pool:
            raise ValueError(f"missing Q21g stratum {phase}:{band}")
        selected.append(min(pool, key=lambda entry: (entry["selection_rank"], entry["id"])))
    if len({entry["id"] for entry in selected}) != 9:
        raise ValueError("Q21i selection contains duplicate positions")
    return selected


def parse_profile(text: str) -> dict:
    fields = {}
    for part in text.strip().split("\t"):
        key, separator, value = part.partition("=")
        if separator:
            fields[key] = value
    required = {
        "bestmove",
        "depth",
        "score_cp",
        "nodes",
        "elapsed_ms",
        "bound",
        "completed_bound",
        "completed_iteration_valid",
        "aborted",
        "abort_reason",
        "static_evaluations",
        "pv_legal",
        "pv_replay_preserves_input",
        "history_matches_expected",
    }
    missing = sorted(required - fields.keys())
    if missing:
        raise ValueError(f"diagnostic output missing: {', '.join(missing)}")
    for key in ("depth", "score_cp", "nodes", "elapsed_ms", "static_evaluations"):
        fields[key] = int(fields[key])
    for key in (
        "completed_iteration_valid",
        "aborted",
        "pv_legal",
        "pv_replay_preserves_input",
        "history_matches_expected",
    ):
        if fields[key] not in {"true", "false"}:
            raise ValueError(f"invalid boolean {key}={fields[key]!r}")
        fields[key] = fields[key] == "true"
    if not all(
        fields[key]
        for key in ("pv_legal", "pv_replay_preserves_input", "history_matches_expected")
    ):
        raise ValueError("diagnostic failed history/PV validation")
    return fields


def parse_max_rss(stderr: str) -> int | None:
    for line in stderr.splitlines():
        if "maximum resident set size" in line:
            value = line.strip().split()[0]
            if value.isdigit():
                return int(value)
    return None


def run_timed(command: list[str], timeout: float, env: dict[str, str]) -> tuple[str, int | None]:
    timed = Path("/usr/bin/time")
    actual = [str(timed), "-l", *command] if sys.platform == "darwin" and timed.is_file() else command
    completed = subprocess.run(
        actual,
        text=True,
        capture_output=True,
        check=False,
        timeout=timeout,
        env=env,
    )
    if completed.returncode:
        raise RuntimeError(
            f"command failed ({completed.returncode}): {' '.join(command)}\n{completed.stderr[-2000:]}"
        )
    return completed.stdout, parse_max_rss(completed.stderr)


def run_search(
    binary: Path,
    weights: Path,
    entry: dict,
    budget: str,
    amount: int,
    arm: str,
) -> dict:
    position = entry["position"]
    command = [str(binary), "--profile-cost", "--sfen", position["initial_sfen"]]
    history = position.get("history_before_usi", [])
    if history:
        command.extend(("--moves", " ".join(history)))
    command.extend(("--expected-sfen", position["sfen"]))
    if budget == "fixed_nodes":
        command.extend(("--nodes", str(amount)))
        timeout = 120.0
    else:
        command.extend(("--time-ms", str(amount)))
        timeout = amount / 1000 + 30.0
    if arm != "material":
        scale = "0" if arm == "cost-only" else "1000"
        command.extend(
            (
                "--weights",
                str(weights),
                "--nnue-output",
                "residual-material",
                "--nnue-residual-scale-permille",
                scale,
            )
        )
    env = dict(os.environ)
    env["RAYON_NUM_THREADS"] = "1"
    stdout, max_rss_bytes = run_timed(command, timeout, env)
    result = parse_profile(stdout)
    result["max_rss_bytes"] = max_rss_bytes
    return result


def run_component(binary: Path, weights: Path, mode: str) -> dict:
    command = [str(binary), "--mode", mode]
    if mode != "material":
        command.extend(("--weights", str(weights)))
    env = dict(os.environ)
    env["RAYON_NUM_THREADS"] = "1"
    stdout, max_rss_bytes = run_timed(command, 120.0, env)
    result = json.loads(stdout)
    if result.get("schema") != "sekirei.nnue-cost-components.v1":
        raise ValueError("unexpected component benchmark schema")
    result["max_rss_bytes"] = max_rss_bytes
    return result


def median(values):
    return statistics.median(values)


def ratio(numerator: float, denominator: float) -> float | None:
    return numerator / denominator if denominator else None


def aggregate(rows: list[dict], components: list[dict]) -> dict:
    position_medians = []
    for budget in ("fixed_nodes", "fixed_time"):
        ids = sorted({row["position_id"] for row in rows if row["budget"] == budget})
        for position_id in ids:
            record = {"budget": budget, "position_id": position_id, "arms": {}}
            for arm in ARMS:
                runs = [
                    row["result"]
                    for row in rows
                    if row["budget"] == budget
                    and row["position_id"] == position_id
                    and row["arm"] == arm
                ]
                if len(runs) != 3:
                    raise ValueError(f"expected three runs for {budget}/{position_id}/{arm}")
                record["arms"][arm] = {
                    key: median([run[key] for run in runs])
                    for key in (
                        "depth",
                        "nodes",
                        "elapsed_ms",
                        "static_evaluations",
                        "max_rss_bytes",
                    )
                }
                record["arms"][arm]["bestmoves"] = [run["bestmove"] for run in runs]
                record["arms"][arm]["scores_cp"] = [run["score_cp"] for run in runs]
                record["arms"][arm]["variability"] = {
                    "unique_bestmoves": len({run["bestmove"] for run in runs}),
                    "unique_scores": len({run["score_cp"] for run in runs}),
                    "depth_range": [
                        min(run["depth"] for run in runs),
                        max(run["depth"] for run in runs),
                    ],
                    "nodes_range": [
                        min(run["nodes"] for run in runs),
                        max(run["nodes"] for run in runs),
                    ],
                }
            position_medians.append(record)

    corpus = {}
    for budget in ("fixed_nodes", "fixed_time"):
        records = [record for record in position_medians if record["budget"] == budget]
        corpus[budget] = {
            arm: {
                key: median([record["arms"][arm][key] for record in records])
                for key in (
                    "depth",
                    "nodes",
                    "elapsed_ms",
                    "static_evaluations",
                    "max_rss_bytes",
                )
            }
            for arm in ARMS
        }
    corpus["fixed_nodes"]["cost_elapsed_ratio_vs_material"] = ratio(
        corpus["fixed_nodes"]["cost-only"]["elapsed_ms"],
        corpus["fixed_nodes"]["material"]["elapsed_ms"],
    )
    corpus["fixed_nodes"]["teacher_elapsed_ratio_vs_cost_only"] = ratio(
        corpus["fixed_nodes"]["teacher"]["elapsed_ms"],
        corpus["fixed_nodes"]["cost-only"]["elapsed_ms"],
    )
    corpus["fixed_time"]["cost_node_ratio_vs_material"] = ratio(
        corpus["fixed_time"]["cost-only"]["nodes"],
        corpus["fixed_time"]["material"]["nodes"],
    )
    corpus["fixed_time"]["teacher_node_ratio_vs_cost_only"] = ratio(
        corpus["fixed_time"]["teacher"]["nodes"],
        corpus["fixed_time"]["cost-only"]["nodes"],
    )

    fixed_rows = [row for row in rows if row["budget"] == "fixed_nodes"]
    identity = []
    for position_id in sorted({row["position_id"] for row in fixed_rows}):
        for repetition in range(3):
            pair = {
                row["arm"]: row["result"]
                for row in fixed_rows
                if row["position_id"] == position_id and row["repetition"] == repetition
            }
            identity.append(
                pair["material"]["bestmove"] == pair["cost-only"]["bestmove"]
                and pair["material"]["score_cp"] == pair["cost-only"]["score_cp"]
            )

    component_summary = {}
    for fixture in ("quiet", "capture", "drop"):
        medians = {}
        for mode in COMPONENT_MODES:
            values = [
                case["median_ns"]
                for record in components
                if record["mode"] == mode
                for case in record["result"]["cases"]
                if case["name"] == fixture
            ]
            if len(values) != 3:
                raise ValueError(f"expected three component runs for {mode}/{fixture}")
            medians[mode] = median(values)
        component_summary[fixture] = {
            **medians,
            "incremental_update_delta_ns": medians["nnue-update"] - medians["material"],
            "incremental_update_ratio": ratio(medians["nnue-update"], medians["material"]),
        }

    forward_ns = median(
        [component_summary[fixture]["forward"] for fixture in component_summary]
    )
    attribution = []
    for record in position_medians:
        if record["budget"] != "fixed_nodes":
            continue
        material = record["arms"]["material"]
        cost_only = record["arms"]["cost-only"]
        estimated_forward_ms = cost_only["static_evaluations"] * forward_ns / 1_000_000
        elapsed_delta_ms = cost_only["elapsed_ms"] - material["elapsed_ms"]
        attribution.append(
            {
                "position_id": record["position_id"],
                "elapsed_delta_ms": elapsed_delta_ms,
                "estimated_forward_ms": estimated_forward_ms,
                "mixed_remainder_ms": elapsed_delta_ms - estimated_forward_ms,
            }
        )

    fixed_time_variability = {}
    for arm in ARMS:
        records = [record for record in position_medians if record["budget"] == "fixed_time"]
        fixed_time_variability[arm] = {
            "positions_with_bestmove_variation": sum(
                record["arms"][arm]["variability"]["unique_bestmoves"] > 1
                for record in records
            ),
            "positions_with_score_variation": sum(
                record["arms"][arm]["variability"]["unique_scores"] > 1
                for record in records
            ),
            "median_node_range": median(
                record["arms"][arm]["variability"]["nodes_range"][1]
                - record["arms"][arm]["variability"]["nodes_range"][0]
                for record in records
            ),
        }

    return {
        "schema": "sekirei.q21i-nnue-cost-summary.v1",
        "diagnostic_only": True,
        "strength_claim": "not_permitted",
        "aggregation": "median of 3 repetitions per position, then median across 9 positions",
        "zero_scale_fixed_node_identity": {
            "matching": sum(identity),
            "comparisons": len(identity),
        },
        "position_medians": position_medians,
        "corpus_medians": corpus,
        "components": component_summary,
        "forward_attribution": {
            "forward_ns_used": forward_ns,
            "method": "cost-only fixed-node static evaluations times component forward median",
            "caveat": "mixed remainder includes accumulator traffic, cache effects, timing noise, and other evaluator-path overhead",
            "median_elapsed_delta_ms": median(item["elapsed_delta_ms"] for item in attribution),
            "median_estimated_forward_ms": median(
                item["estimated_forward_ms"] for item in attribution
            ),
            "median_mixed_remainder_ms": median(
                item["mixed_remainder_ms"] for item in attribution
            ),
            "positions": attribution,
        },
        "fixed_time_variability": fixed_time_variability,
    }


def git_metadata(root: Path) -> dict:
    revision = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=root, text=True, capture_output=True, check=True
    ).stdout.strip()
    dirty = bool(
        subprocess.run(
            ["git", "status", "--porcelain"], cwd=root, text=True, capture_output=True, check=True
        ).stdout.strip()
    )
    return {"revision": revision, "dirty": dirty}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--search-binary", type=Path, required=True)
    parser.add_argument("--component-binary", type=Path, required=True)
    parser.add_argument("--weights", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--nodes", type=int, default=100_000)
    parser.add_argument("--time-ms", type=int, default=1_000)
    args = parser.parse_args()
    if args.output_dir.exists():
        parser.error("output-dir already exists")
    for path in (args.corpus, args.search_binary, args.component_binary, args.weights):
        if not path.is_file():
            parser.error(f"missing input: {path}")
    if args.nodes <= 0 or args.time_ms <= 0:
        parser.error("nodes and time-ms must be positive")

    corpus = json.loads(args.corpus.read_text(encoding="utf-8"))
    selected = select_positions(corpus.get("entries", []))
    root = Path(__file__).resolve().parent.parent
    preregistration = {
        "schema": "sekirei.q21i-nnue-cost-preregistration.v1",
        "diagnostic_only": True,
        "strength_claim": "not_permitted",
        "factor": "same_time_nnue_cost",
        "selection": "minimum selection_rank within alternating normal/tactical class per phase/material stratum",
        "position_ids": [entry["id"] for entry in selected],
        "strata": [
            {
                "id": entry["id"],
                "phase": entry["attributes"]["phase"],
                "material_band": entry["attributes"]["material_band"],
                "selection_class": entry["selection_class"],
            }
            for entry in selected
        ],
        "contract": {
            "fixed_nodes": args.nodes,
            "fixed_time_ms": args.time_ms,
            "repetitions": 3,
            "rayon_threads": 1,
            "arms": {
                "material": {"weights": None},
                "cost-only": {"nnue_output": "residual-material", "scale_permille": 0},
                "teacher": {"nnue_output": "residual-material", "scale_permille": 1000},
            },
            "search_orders": SEARCH_ORDERS,
            "component_orders": COMPONENT_ORDERS,
            "aggregation": "median of repetitions per position, then corpus median",
        },
        "inputs": {
            "corpus": str(args.corpus),
            "corpus_sha256": sha256(args.corpus),
            "weights": str(args.weights),
            "weights_sha256": sha256(args.weights),
            "search_binary": str(args.search_binary),
            "search_binary_sha256": sha256(args.search_binary),
            "component_binary": str(args.component_binary),
            "component_binary_sha256": sha256(args.component_binary),
            "orchestrator_source_sha256": sha256(Path(__file__).resolve()),
            "component_source_sha256": sha256(
                root / "crates/sekirei-bench/src/bin/nnue_cost_components.rs"
            ),
        },
        "git": git_metadata(root),
    }
    args.output_dir.mkdir(parents=True)
    prereg_path = args.output_dir / "preregistration.json"
    prereg_path.write_text(
        json.dumps(preregistration, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )

    rows = []
    for budget, amount in (("fixed_nodes", args.nodes), ("fixed_time", args.time_ms)):
        for repetition, order in enumerate(SEARCH_ORDERS):
            for entry in selected:
                for arm in order:
                    rows.append(
                        {
                            "budget": budget,
                            "amount": amount,
                            "repetition": repetition,
                            "position_id": entry["id"],
                            "phase": entry["attributes"]["phase"],
                            "material_band": entry["attributes"]["material_band"],
                            "selection_class": entry["selection_class"],
                            "arm": arm,
                            "result": run_search(
                                args.search_binary, args.weights, entry, budget, amount, arm
                            ),
                        }
                    )

    components = []
    for repetition, order in enumerate(COMPONENT_ORDERS):
        for mode in order:
            components.append(
                {
                    "repetition": repetition,
                    "mode": mode,
                    "result": run_component(args.component_binary, args.weights, mode),
                }
            )
    profile = {
        "schema": "sekirei.q21i-nnue-cost-profile.v1",
        "diagnostic_only": True,
        "strength_claim": "not_permitted",
        "preregistration_sha256": sha256(prereg_path),
        "rows": rows,
        "component_runs": components,
    }
    profile_path = args.output_dir / "profile.json"
    profile_path.write_text(json.dumps(profile, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    summary = aggregate(rows, components)
    summary["profile_sha256"] = sha256(profile_path)
    (args.output_dir / "summary.json").write_text(
        json.dumps(summary, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
