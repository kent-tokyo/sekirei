#!/usr/bin/env python3
"""Tests for Q21o teacher-contract preparation and decision."""

import argparse
import json
import tempfile
from pathlib import Path

import finalize_q21o_teacher_contract as finalizer
import prepare_q21o_teacher_contract as preparer


def write(path: Path, value: object) -> None:
    path.write_text(json.dumps(value), encoding="utf-8")


def result(score: int, bestmove: str, elapsed: int) -> dict:
    return {
        "completion": "search_completed",
        "completed_iteration_valid": "true",
        "completed_bound": "exact",
        "pv_legal": True,
        "history_matches_expected": "true",
        "score_cp": score,
        "bestmove": bestmove,
        "depth": 7,
        "elapsed_ms": elapsed,
    }


def main() -> None:
    assert finalizer.regret_class(99) == "small"
    assert finalizer.regret_class(100) == "moderate"
    assert finalizer.regret_class(300) == "major"
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        paths = {name: root / f"{name}.json" for name in (
            "decision", "summary", "shallow", "corpus", "depth7", "prereg", "measurements"
        )}
        binary, weights, runner, finalize_tool = (
            root / "engine", root / "weights", root / "runner.py", root / "finalizer.py"
        )
        for path in (binary, weights, runner, finalize_tool):
            path.write_bytes(path.name.encode())
        problems = ["problem-opening", "problem-middle", "problem-end"]
        controls = ["control-opening", "control-middle", "control-end"]
        write(paths["decision"], {
            "schema": "sekirei.q21n-final-decision.v1", "status": "complete",
            "classification": "teacher_depth_insufficient",
            "rows": [{"position_id": identifier} for identifier in problems],
        })
        summary_rows = []
        for phase, problem, control in zip(("opening", "middlegame", "endgame"), problems, controls):
            summary_rows.extend((
                {
                    "position_id": problem, "phase": phase, "material_band": "balanced",
                    "forcing_class": "forcing_attack", "ordinary_cp_comparable": True,
                    "deep_regret_cp": 150, "shallow_top_moves": ["b"],
                },
                {
                    "position_id": control, "phase": phase, "material_band": "balanced",
                    "forcing_class": "quiet", "ordinary_cp_comparable": True,
                    "deep_regret_cp": 10, "shallow_top_moves": ["b"],
                },
            ))
        write(paths["summary"], {"schema": "sekirei.q21n-teacher-depth-audit.v1", "rows": summary_rows})
        all_ids = problems + controls
        write(paths["corpus"], {"positions": [{"id": identifier} for identifier in all_ids]})
        write(paths["shallow"], {
            "rows": [
                {
                    "id": identifier,
                    "teacher_root": {"root_candidates": [
                        {"move": "b", "score_cp": 100},
                        {"move": "a", "score_cp": 0},
                    ]},
                }
                for identifier in all_ids
            ],
        })
        write(paths["depth7"], {"rows": [
            {"free": {"nodes": 331_022}},
            {"free": {"nodes": 3_161_899}},
            {"free": {"nodes": 15_778_662}},
        ]})
        prereg = preparer.prepare(argparse.Namespace(
            q21n_decision=paths["decision"], q21n_summary=paths["summary"],
            shallow=paths["shallow"], corpus=paths["corpus"],
            depth7_measurements=paths["depth7"], binary=binary, weights=weights,
            runner=runner, finalizer=finalize_tool, control_seed=2121,
        ))
        assert len(prereg["selection"]["parents"]) == 6
        assert prereg["arms"]["nodes3200k"]["nodes"] == 3_200_000
        write(paths["prereg"], prereg)
        measured_rows = []
        for selection in prereg["selection"]["parents"]:
            is_problem = selection["role"] == "problem"
            old_score = 100 if is_problem else 450
            calibration = {}
            for arm, elapsed in (("depth7", 200), ("nodes3200k", 100)):
                calibration[arm] = [
                    {
                        "free": result(500, "a", elapsed),
                        "fixed_depth3_top": {"b": result(old_score, "b", elapsed // 2)},
                    }
                    for _ in range(2)
                ]
            ranking = {
                arm: {"a": result(500, "a", 10), "b": result(old_score, "b", 10)}
                for arm in ("depth7", "nodes3200k")
            }
            measured_rows.append({
                "selection": selection, "candidate_moves": ["a", "b"],
                "calibration": calibration, "ranking": ranking,
            })
        write(paths["measurements"], {
            "schema": "sekirei.q21o-teacher-contract-measurements.v1",
            "preregistration": {"sha256": finalizer.sha256(paths["prereg"])},
            "rows": measured_rows,
        })
        decision = finalizer.finalize(argparse.Namespace(
            preregistration=paths["prereg"], measurements=paths["measurements"],
        ))
        assert decision["status"] == "pass"
        assert decision["selected_contract"]["arm"] == "nodes3200k"
        assert decision["label_reaudit"]["old_depth3_major_depth7_regret_ge_300cp"] == 3
        assert decision["label_reaudit"]["selected_label_major_depth7_regret_ge_300cp"] == 0
        assert decision["q20_authorized"] is False
        rendered = finalizer.report(decision)
        assert "Selected: `nodes3200k`" in rendered
        assert "Q20 remain unauthorized" in rendered
    print("test_q21o_teacher_contract: PASS")


if __name__ == "__main__":
    main()
