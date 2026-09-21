#!/usr/bin/env python3
"""Evaluate the preregistered Q21k exact NNUE evaluation-cache screen."""

from __future__ import annotations

import argparse
import hashlib
import json
import statistics
from pathlib import Path


IDENTITY_FIELDS = (
    "bestmove",
    "depth",
    "score_cp",
    "nodes",
    "bound",
    "completed_bound",
    "completed_iteration_valid",
    "aborted",
    "abort_reason",
    "static_evaluations",
    "pv_usi",
    "pv_legal",
    "pv_replay_preserves_input",
    "history_final_hash",
    "history_matches_expected",
)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read_json(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def fixed_node_rows(profile: dict) -> dict[tuple[int, str, str], dict]:
    return {
        (row["repetition"], row["position_id"], row["arm"]): row["result"]
        for row in profile.get("rows", [])
        if row.get("budget") == "fixed_nodes"
    }


def identity(candidate: dict, baseline: dict) -> dict:
    candidate_rows = fixed_node_rows(candidate)
    baseline_rows = fixed_node_rows(baseline)
    if candidate_rows.keys() != baseline_rows.keys():
        raise ValueError("candidate/baseline fixed-node matrices differ")
    mismatches = []
    for key in sorted(candidate_rows):
        for field in IDENTITY_FIELDS:
            if candidate_rows[key].get(field) != baseline_rows[key].get(field):
                mismatches.append(
                    {
                        "key": list(key),
                        "field": field,
                        "baseline": baseline_rows[key].get(field),
                        "candidate": candidate_rows[key].get(field),
                    }
                )
    return {
        "rows": len(candidate_rows),
        "fields_per_row": len(IDENTITY_FIELDS),
        "comparisons": len(candidate_rows) * len(IDENTITY_FIELDS),
        "mismatch_count": len(mismatches),
        "mismatches": mismatches[:20],
    }


def median_baseline(summaries: list[dict], budget: str, field: str) -> float:
    return float(
        statistics.median(
            summary["corpus_medians"][budget]["cost-only"][field]
            for summary in summaries
        )
    )


def evaluate(prereg: dict, baselines: list[tuple[dict, dict, dict]], candidate: tuple[dict, dict, dict]) -> dict:
    if prereg.get("schema") != "sekirei.q21k-eval-cache-preregistration.v1":
        raise ValueError("unsupported Q21k preregistration")
    if len(baselines) != 2:
        raise ValueError("Q21k requires exactly two A/A baselines")
    candidate_prereg, candidate_profile, candidate_summary = candidate
    baseline_summaries = [summary for _, _, summary in baselines]

    expected_positions = candidate_prereg.get("position_ids")
    expected_contract = candidate_prereg.get("contract")
    expected_inputs = candidate_prereg.get("inputs", {})
    for baseline_prereg, _, _ in baselines:
        if baseline_prereg.get("position_ids") != expected_positions:
            raise ValueError("baseline position IDs differ")
        if baseline_prereg.get("contract") != expected_contract:
            raise ValueError("baseline measurement contract differs")
        for key in ("corpus_sha256", "weights_sha256"):
            if baseline_prereg.get("inputs", {}).get(key) != expected_inputs.get(key):
                raise ValueError(f"baseline {key} differs")

    identity_checks = [identity(candidate_profile, profile) for _, profile, _ in baselines]

    cache_rows = [
        row["result"]
        for row in candidate_profile.get("rows", [])
        if row.get("arm") in ("cost-only", "teacher")
    ]
    material_rows = [
        row["result"] for row in candidate_profile.get("rows", []) if row.get("arm") == "material"
    ]
    probes = sum(int(row.get("eval_cache_probes", -1)) for row in cache_rows)
    hits = sum(int(row.get("eval_cache_hits", -1)) for row in cache_rows)
    hit_rate = hits / probes if probes > 0 else 0.0
    request_identity = all(
        int(row.get("eval_cache_probes", -1)) == int(row.get("static_evaluations", -2))
        for row in cache_rows
    )
    material_disabled = all(
        int(row.get("eval_cache_probes", -1)) == 0
        and int(row.get("eval_cache_hits", -1)) == 0
        for row in material_rows
    )

    baseline_nodes = median_baseline(baseline_summaries, "fixed_time", "nodes")
    candidate_nodes = float(
        candidate_summary["corpus_medians"]["fixed_time"]["cost-only"]["nodes"]
    )
    node_improvement = candidate_nodes / baseline_nodes - 1.0
    baseline_elapsed = median_baseline(baseline_summaries, "fixed_nodes", "elapsed_ms")
    candidate_elapsed = float(
        candidate_summary["corpus_medians"]["fixed_nodes"]["cost-only"]["elapsed_ms"]
    )
    elapsed_improvement = baseline_elapsed / candidate_elapsed - 1.0
    baseline_rss = median_baseline(baseline_summaries, "fixed_time", "max_rss_bytes")
    candidate_rss = float(
        candidate_summary["corpus_medians"]["fixed_time"]["cost-only"]["max_rss_bytes"]
    )
    additional_rss = max(0.0, candidate_rss - baseline_rss)

    contract = prereg["contract"]
    checks = {
        "fixed_node_identity": all(item["mismatch_count"] == 0 for item in identity_checks),
        "static_evaluation_request_identity": request_identity,
        "cache_hit_rate": hit_rate >= float(contract["minimum_cache_hit_rate"]),
        "search_improvement": max(node_improvement, elapsed_improvement)
        >= float(contract["minimum_search_improvement"]),
        "material_cache_disabled": material_disabled,
        "memory_budget": additional_rss <= float(contract["maximum_additional_cache_bytes"]),
    }
    passed = all(checks.values())
    return {
        "schema": "sekirei.q21k-eval-cache-decision.v1",
        "diagnostic_only": True,
        "strength_claim": False,
        "single_factor": prereg["single_factor"],
        "identity_against_aa_baselines": identity_checks,
        "cache": {"probes": probes, "hits": hits, "hit_rate": hit_rate},
        "performance": {
            "baseline_fixed_time_nodes_median": baseline_nodes,
            "candidate_fixed_time_nodes": candidate_nodes,
            "fixed_time_node_improvement": node_improvement,
            "baseline_fixed_node_elapsed_ms_median": baseline_elapsed,
            "candidate_fixed_node_elapsed_ms": candidate_elapsed,
            "fixed_node_elapsed_improvement": elapsed_improvement,
        },
        "memory": {
            "baseline_fixed_time_rss_bytes_median": baseline_rss,
            "candidate_fixed_time_rss_bytes": candidate_rss,
            "additional_rss_bytes": additional_rss,
            "maximum_additional_cache_bytes": contract["maximum_additional_cache_bytes"],
        },
        "checks": checks,
        "status": "screen_pass" if passed else "screen_fail",
        "next_action": (
            "freeze_cost_track_and_start_content_track"
            if passed
            else "reject_eval_cache_factor"
        ),
    }


def artifact(directory: Path) -> tuple[dict, dict, dict]:
    return tuple(
        read_json(directory / name)
        for name in ("preregistration.json", "profile.json", "summary.json")
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preregistration", type=Path, required=True)
    parser.add_argument("--baseline", type=Path, action="append", required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = evaluate(
            read_json(args.preregistration),
            [artifact(path) for path in args.baseline],
            artifact(args.candidate),
        )
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        parser.error(str(error))
    result["inputs"] = {
        "preregistration": {
            "path": str(args.preregistration),
            "sha256": sha256(args.preregistration),
        },
        "baselines": [
            {"path": str(path), "summary_sha256": sha256(path / "summary.json")}
            for path in args.baseline
        ],
        "candidate": {
            "path": str(args.candidate),
            "profile_sha256": sha256(args.candidate / "profile.json"),
            "summary_sha256": sha256(args.candidate / "summary.json"),
        },
    }
    args.output.write_text(
        json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps({"status": result["status"], **result["checks"]}, sort_keys=True))
    return 0 if result["status"] == "screen_pass" else 1


if __name__ == "__main__":
    raise SystemExit(main())
