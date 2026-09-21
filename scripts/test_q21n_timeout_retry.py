#!/usr/bin/env python3
"""Tests for Q21n's timeout-only retry and merge contract."""

import argparse
import json
import tempfile
from pathlib import Path

import merge_q21n_teacher_depth_retry as merger
import prepare_q21n_timeout_exclusion as exclusion_preparer
import prepare_q21n_timeout_retry as preparer


def write(path: Path, value: object) -> None:
    path.write_text(json.dumps(value), encoding="utf-8")


def complete_row(identifier: str) -> dict:
    return {
        "id": identifier,
        "candidate_prefix_complete": True,
        "complete_legal_root_set": True,
        "teacher_root": {
            "root_legal_move_count": 1,
            "root_candidates": [
                {"move": "7g7f", "score_cp": 0, "depth": 5, "bound": "exact", "abort_reason": "none"}
            ],
        },
    }


def main() -> None:
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        prereg, corpus, initial = root / "prereg.json", root / "corpus.json", root / "initial.json"
        retry_corpus, amendment = root / "retry-corpus.json", root / "amendment.json"
        retry_deep = root / "retry-deep.json"
        categories = [
            f"{phase}/{material}"
            for phase in ("opening", "middlegame", "endgame")
            for material in ("stm_behind", "balanced", "stm_ahead")
            for _ in range(2)
        ]
        positions = [
            {"id": f"p{index:02d}", "category": category}
            for index, category in enumerate(categories)
        ]
        write(corpus, {"positions": positions})
        write(prereg, {
            "schema": "sekirei.q21n-teacher-depth-preregistration.v1",
            "status": "frozen_before_deep_labels",
            "inputs": {
                "corpus": {"sha256": preparer.sha256(corpus)},
                "binary": {"sha256": "binary"},
                "weights": {"sha256": "weights"},
            },
            "deep_contract": {"depth": 5, "root_candidate_limit": 600},
        })
        rows = [complete_row(f"p{index:02d}") for index in range(17)]
        rows.append({
            "id": "p17",
            "candidate_prefix_complete": False,
            "complete_legal_root_set": False,
            "teacher_root": {"completion": "timeout", "score_cp": None},
        })
        teacher = {"binary_sha256": "binary", "weights_sha256": "weights", "nnue_output": "residual-material"}
        write(initial, {
            "schema": "sekirei.root-rank-teacher-corpus.v1",
            "contract": {"depth": 5, "root_candidate_limit": 600},
            "teacher": teacher,
            "rows": rows,
        })
        amendment_doc, retry_corpus_doc = preparer.prepare(argparse.Namespace(
            preregistration=prereg, corpus=corpus, initial_deep=initial,
            retry_corpus=retry_corpus,
        ))
        write(amendment, amendment_doc)
        assert retry_corpus_doc["selection"]["score_blind"] is True
        assert amendment_doc["retry"]["position_id"] == "p17"
        assert amendment_doc["retry"]["observed_label_before_retry"] is False
        write(retry_deep, {
            "schema": "sekirei.root-rank-teacher-corpus.v1",
            "contract": {"depth": 5, "root_candidate_limit": 600},
            "teacher": teacher,
            "rows": [complete_row("p17")],
        })
        merged = merger.merge(argparse.Namespace(
            amendment=amendment, initial_deep=initial, retry_deep=retry_deep,
        ))
        assert merged["contract"]["complete_legal_root_set"] is True
        assert len(merged["rows"]) == 18
        assert all(row["complete_legal_root_set"] for row in merged["rows"])

        # A second label-free timeout may be resource-censored, but only when
        # all balanced parents and every phase/material stratum remain.
        write(retry_deep, {
            "schema": "sekirei.root-rank-teacher-corpus.v1",
            "rows": [rows[-1]],
        })
        exclusion = exclusion_preparer.prepare(argparse.Namespace(
            preregistration=prereg, corpus=corpus, initial_deep=initial,
            retry_amendment=amendment, retry_deep=retry_deep,
        ))
        assert exclusion["completion_contract"]["accepted_complete_parents"] == 17
        assert exclusion["completion_contract"]["balanced_parents_retained"] == 6
        assert exclusion["completion_contract"]["substitution_allowed"] is False
    print("test_q21n_timeout_retry: PASS")


if __name__ == "__main__":
    main()
