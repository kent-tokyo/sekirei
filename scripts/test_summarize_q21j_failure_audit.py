#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
import json
import tempfile
from pathlib import Path


SCRIPT = Path(__file__).with_name("summarize_q21j_failure_audit.py")
SPEC = importlib.util.spec_from_file_location("summarize_q21j", SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MODULE)


def result(bestmove: str, score: int, nodes: int, depth: int = 5) -> dict:
    return {
        "bestmove": bestmove,
        "score_cp": score,
        "nodes": nodes,
        "depth": depth,
        "elapsed_ms": 10,
        "static_evaluations": nodes // 2,
        "completed_iteration_valid": True,
        "pv_legal": True,
        "pv_replay_preserves_input": True,
        "history_matches_expected": True,
    }


def main() -> None:
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        inputs = {}
        for name in ("binary", "candidate", "teacher", "runner_source"):
            path = root / name
            path.write_text(name, encoding="utf-8")
            inputs[name] = {"path": str(path), "sha256": MODULE.sha256(path)}
        positions = []
        ids = [f"p{i:02d}" for i in range(32)]
        for index, position_id in enumerate(ids):
            positions.append({
                "id": position_id,
                "game_num": index + 1,
                "game_result": "baseline_win" if index < 25 else "candidate_win",
                "selection": {"rule": "first_candidate_score_drop_ge_300cp"},
                "position": {"actual_move_usi": "7g7f"},
            })
        corpus = {
            "schema": "sekirei.q21j-failure-audit-corpus.v1",
            "counts": {"first_large_drop": 30},
            "positions": positions,
        }
        corpus_path = root / "corpus.json"
        corpus_path.write_text(json.dumps(corpus), encoding="utf-8")
        inputs["corpus"] = {"path": str(corpus_path), "sha256": MODULE.sha256(corpus_path)}
        prereg = {
            "schema": "sekirei.q21j-failure-audit-preregistration.v1",
            "inputs": inputs,
            "position_ids": ids,
            "contract": {
                "attribution_boundary": "no causal percentage",
                "fixed_nodes": 100_000,
                "fixed_time_ms": 1_000,
            },
        }
        prereg_path = root / "prereg.json"
        prereg_path.write_text(json.dumps(prereg), encoding="utf-8")

        rows = []
        for index, position_id in enumerate(ids):
            content = index < 16
            mate = index == 0
            for repetition in range(3):
                for arm in MODULE.ARMS:
                    score = 100
                    move = "7g7f"
                    if arm == "candidate" and content:
                        score = 900_000 if mate else 500
                        move = "2g2f"
                    rows.append({
                        "cell": "fixed_nodes_free", "repetition": repetition,
                        "position_id": position_id, "arm": arm,
                        "amount": 100_000, "game_num": index + 1,
                        "game_result": "baseline_win" if index < 25 else "candidate_win",
                        "actual_move_usi": "7g7f",
                        "result": result(move, score, 100_000),
                    })
            for arm in MODULE.ARMS:
                free_score = 900_000 if arm == "candidate" and mate else (500 if arm == "candidate" and content else 100)
                forced_score = free_score if mate else free_score - (400 if arm == "candidate" and content else 100)
                rows.append({
                    "cell": "fixed_nodes_actual", "repetition": 0,
                    "position_id": position_id, "arm": arm,
                    "amount": 100_000, "game_num": index + 1,
                    "game_result": "baseline_win" if index < 25 else "candidate_win",
                    "actual_move_usi": "7g7f",
                    "result": result("7g7f", forced_score, 100_000),
                })
            for repetition in range(3):
                for arm in MODULE.ARMS:
                    nodes = {"material": 1_000, "candidate-cost-only": 600,
                             "candidate": 580, "teacher": 550}[arm]
                    rows.append({
                        "cell": "fixed_time_free", "repetition": repetition,
                        "position_id": position_id, "arm": arm,
                        "amount": 1_000, "game_num": index + 1,
                        "game_result": "baseline_win" if index < 25 else "candidate_win",
                        "actual_move_usi": "7g7f",
                        "result": result("7g7f", 100, nodes),
                    })
        measurements_path = root / "measurements.jsonl"
        measurements_path.write_text("".join(json.dumps(row) + "\n" for row in rows), encoding="utf-8")
        manifest = {
            "schema": "sekirei.q21j-failure-audit-measurements.v1",
            "status": "complete",
            "measurements": 896,
            "expected_measurements": 896,
            "preregistration_sha256": MODULE.sha256(prereg_path),
            "measurements_sha256": MODULE.sha256(measurements_path),
        }
        summary = MODULE.summarize(corpus, prereg, manifest, rows, prereg_path, measurements_path)
        assert summary["validation"]["measurements"] == 896
        assert summary["cost_isolation"]["raw_fixed_node_score_and_bestmove_matches"] == 96
        assert summary["cost_isolation"]["pass"] is True
        assert summary["fixed_time_cost"]["candidate_cost_only_material_node_ratio"]["median"] == 0.6
        assert summary["conclusion"]["cost_pressure"] is True
        assert summary["fixed_node_content"]["candidate_vs_material"]["bestmove_disagreements"] == 16
        assert summary["fixed_node_content"]["candidate_vs_material"]["mate_like_positions_excluded_from_cp_metrics"] == 1
        assert summary["fixed_node_content"]["candidate_vs_material"]["large_score_differences_ge_300cp"] == 15
        assert summary["actual_move_regret"]["all"]["candidate"]["major_regret_ge_300cp"] == 15
        assert summary["signals"]["classification_counts"] == {"both": 16, "cost_only": 16}
        assert summary["candidate_remains_rejected"] is True
        assert summary["q20_authorized"] is False


if __name__ == "__main__":
    main()
