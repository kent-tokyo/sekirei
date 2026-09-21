#!/usr/bin/env python3
"""Tests for Q21n's targeted depth-7 adjudication."""

import argparse
import json
import tempfile
from pathlib import Path

import finalize_q21n_depth7_adjudication as finalizer
import prepare_q21n_depth7_adjudication as preparer


def write(path: Path, value: object) -> None:
    path.write_text(json.dumps(value), encoding="utf-8")


def result(score: int, bestmove: str) -> dict:
    return {
        "completion": "search_completed",
        "completed_iteration_valid": "true",
        "completed_bound": "exact",
        "pv_legal": True,
        "history_matches_expected": "true",
        "score_cp": score,
        "bestmove": bestmove,
    }


def main() -> None:
    assert finalizer.classify([0, 99], False) == "student_reproduction_primary"
    assert finalizer.classify([100], False) == "mixed_teacher_and_student_risk"
    assert finalizer.classify([300], False) == "teacher_depth_insufficient"
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        summary, corpus = root / "summary.json", root / "corpus.json"
        binary, weights, runner, finalize_tool = (
            root / "engine", root / "weights", root / "runner.py", root / "finalizer.py"
        )
        for path in (binary, weights, runner, finalize_tool):
            path.write_bytes(path.name.encode())
        selected = [
            {
                "position_id": f"p{index}", "category": "middlegame/balanced",
                "forcing_class": "forcing_attack", "ordinary_cp_comparable": True,
                "deep_regret_cp": 100 + index, "shallow_top_moves": ["7g7f"],
            }
            for index in range(3)
        ]
        write(summary, {
            "schema": "sekirei.q21n-teacher-depth-audit.v1",
            "status": "complete",
            "classification": "mixed_teacher_and_student_risk",
            "overall": {"moderate_regret_ge_100cp": 3},
            "rows": selected,
        })
        write(corpus, {"positions": [{"id": f"p{index}"} for index in range(3)]})
        prereg = preparer.prepare(argparse.Namespace(
            summary=summary, corpus=corpus, binary=binary, weights=weights,
            runner=runner, finalizer=finalize_tool,
        ))
        prereg_path, measurements_path = root / "prereg.json", root / "measurements.json"
        write(prereg_path, prereg)
        rows = []
        for index, selection in enumerate(prereg["selected"]):
            rows.append({
                "selection": selection,
                "free": result(200, "3c3d"),
                "fixed_depth3_top": {"7g7f": result(150 - index * 10, "7g7f")},
            })
        write(measurements_path, {
            "schema": "sekirei.q21n-depth7-adjudication-measurements.v1",
            "preregistration": {"sha256": finalizer.sha256(prereg_path)},
            "rows": rows,
        })
        decision = finalizer.finalize(argparse.Namespace(
            preregistration=prereg_path, measurements=measurements_path,
        ))
        assert decision["classification"] == "student_reproduction_primary"
        assert decision["summary"]["maximum_regret_cp"] == 70
        assert decision["q20_authorized"] is False
    print("test_q21n_depth7_adjudication: PASS")


if __name__ == "__main__":
    main()
