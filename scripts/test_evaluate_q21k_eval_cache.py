#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
from pathlib import Path


ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "q21k_eval_cache", ROOT / "evaluate_q21k_eval_cache.py"
)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def preregistration() -> dict:
    return {
        "schema": "sekirei.q21k-eval-cache-preregistration.v1",
        "single_factor": "cache",
        "contract": {
            "minimum_cache_hit_rate": 0.10,
            "minimum_search_improvement": 0.01,
            "maximum_additional_cache_bytes": 2_097_152,
        },
    }


def run_preregistration() -> dict:
    return {
        "position_ids": ["p"],
        "contract": {"fixed_nodes": 100, "fixed_time_ms": 1_000},
        "inputs": {"corpus_sha256": "c", "weights_sha256": "w"},
    }


def result(probes: int, hits: int) -> dict:
    return {
        "bestmove": "7g7f",
        "depth": 4,
        "score_cp": 10,
        "nodes": 100,
        "bound": "exact",
        "completed_bound": "exact",
        "completed_iteration_valid": True,
        "aborted": False,
        "abort_reason": "none",
        "static_evaluations": probes,
        "eval_cache_probes": probes,
        "eval_cache_hits": hits,
        "pv_usi": ["7g7f"],
        "pv_legal": True,
        "pv_replay_preserves_input": True,
        "history_final_hash": 1,
        "history_matches_expected": True,
    }


def artifact(nodes: int, elapsed: int, rss: int, cached: bool) -> tuple[dict, dict, dict]:
    rows = []
    for arm in ("material", "cost-only", "teacher"):
        probes = 0 if arm == "material" else 100
        hits = 0 if arm == "material" or not cached else 20
        rows.append(
            {
                "budget": "fixed_nodes",
                "repetition": 0,
                "position_id": "p",
                "arm": arm,
                "result": result(probes, hits),
            }
        )
        rows.append(
            {
                "budget": "fixed_time",
                "repetition": 0,
                "position_id": "p",
                "arm": arm,
                "result": result(probes, hits),
            }
        )
    summary = {
        "corpus_medians": {
            "fixed_nodes": {"cost-only": {"elapsed_ms": elapsed}},
            "fixed_time": {"cost-only": {"nodes": nodes, "max_rss_bytes": rss}},
        }
    }
    return run_preregistration(), {"rows": rows}, summary


def test_passes_only_when_every_preregistered_condition_passes() -> None:
    baselines = [artifact(1_000, 100, 10_000_000, False)] * 2
    candidate = artifact(1_050, 95, 11_000_000, True)
    decision = MODULE.evaluate(preregistration(), baselines, candidate)
    assert decision["status"] == "screen_pass"
    assert all(decision["checks"].values())

    fixed_node_cost_row = next(
        row
        for row in candidate[1]["rows"]
        if row["budget"] == "fixed_nodes" and row["arm"] == "cost-only"
    )
    fixed_node_cost_row["result"]["score_cp"] = 11
    decision = MODULE.evaluate(preregistration(), baselines, candidate)
    assert decision["status"] == "screen_fail"
    assert decision["checks"]["fixed_node_identity"] is False


if __name__ == "__main__":
    test_passes_only_when_every_preregistered_condition_passes()
    print("PASS")
