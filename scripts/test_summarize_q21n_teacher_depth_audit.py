#!/usr/bin/env python3
"""Tests for summarize_q21n_teacher_depth_audit.py."""

import argparse
import json
import tempfile
from pathlib import Path

import summarize_q21n_teacher_depth_audit as module


def write(path: Path, value: object) -> None:
    path.write_text(json.dumps(value), encoding="utf-8")


def teacher_row(index: int, depth: int, reversed_order: bool) -> dict:
    first, second = ((0, 400) if reversed_order else (100, 0))
    return {
        "id": f"p{index:02d}",
        "category": f"{('opening', 'middlegame', 'endgame')[index // 6]}/"
                    f"{('stm_behind', 'balanced', 'stm_ahead')[(index // 2) % 3]}",
        "candidate_prefix_complete": True,
        "complete_legal_root_set": True,
        "teacher_root": {
            "root_legal_move_count": 2,
            "root_candidates": [
                {"move": "7g7f", "score_cp": first, "depth": depth, "bound": "exact", "abort_reason": "none"},
                {"move": "3c3d", "score_cp": second, "depth": depth, "bound": "exact", "abort_reason": "none"},
            ],
        },
    }


def main() -> None:
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        prereg, shallow, deep, forcing, corpus, exclusion = (
            root / "prereg.json", root / "shallow.json", root / "deep.json",
            root / "forcing.json", root / "corpus.json", root / "exclusion.json",
        )
        write(corpus, {"positions": 18})
        teacher = {"binary_sha256": "binary", "weights_sha256": "weights", "nnue_output": "residual-material"}
        shallow_document = {
            "schema": "sekirei.root-rank-teacher-corpus.v1",
            "teacher": teacher,
            "rows": [teacher_row(index, 3, False) for index in range(18)],
        }
        write(shallow, shallow_document)
        write(forcing, {
            "entries": [
                {"id": f"p{index:02d}", "forcing_class": "forcing_attack" if index < 9 else "quiet"}
                for index in range(18)
            ],
        })
        write(prereg, {
            "schema": "sekirei.q21n-teacher-depth-preregistration.v1",
            "status": "frozen_before_deep_labels",
            "inputs": {
                "corpus": {"sha256": module.sha256(corpus)},
                "shallow": {"sha256": module.sha256(shallow)},
                "forcing": {"sha256": module.sha256(forcing)},
            },
            "selection": {"positions": 18},
            "deep_contract": {"depth": 5, "root_candidate_limit": 600},
            "metrics": {"ordinary_cp_limit": 10_000},
            "decision_rule": {"minimum_ordinary_positions": 12},
        })
        deep_rows = [teacher_row(index, 5, index < 5) for index in range(17)]
        deep_rows.append({
            "id": "p17", "category": "endgame/stm_ahead",
            "candidate_prefix_complete": False, "complete_legal_root_set": False,
            "teacher_root": {"completion": "timeout"},
        })
        write(deep, {
            "schema": "sekirei.root-rank-teacher-corpus.v1",
            "contract": {"depth": 5, "root_candidate_limit": 600, "complete_legal_root_set": False},
            "teacher": teacher,
            "source_corpus": {"sha256": module.sha256(corpus)},
            "rows": deep_rows,
        })
        write(exclusion, {
            "schema": "sekirei.q21n-timeout-exclusion.v1",
            "status": "frozen_before_completed-score-inspection",
            "reason": "synthetic unlabeled timeout",
            "inputs": {"initial_deep": {"sha256": module.sha256(deep)}},
            "completion_contract": {
                "accepted_complete_parents": 17,
                "resource_censored_parents": 1,
                "excluded_position_id": "p17",
            },
        })
        document = module.audit(argparse.Namespace(
            preregistration=prereg, shallow=shallow, deep=deep, forcing=forcing,
            timeout_exclusion=exclusion,
        ))
        assert document["status"] == "complete"
        assert document["overall"]["major_regret_ge_300cp"] == 5
        assert document["classification"] == "teacher_depth_insufficient"
        assert document["development_match_authorized"] is False
        assert document["q20_authorized"] is False
        assert "depth-5" in document["reference_boundary"]
        assert document["coverage"]["resource_censored_parents"] == 1
    print("test_summarize_q21n_teacher_depth_audit: PASS")


if __name__ == "__main__":
    main()
