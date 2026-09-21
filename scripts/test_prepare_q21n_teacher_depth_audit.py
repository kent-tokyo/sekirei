#!/usr/bin/env python3
"""Tests for prepare_q21n_teacher_depth_audit.py."""

import argparse
import json
import tempfile
from pathlib import Path

import prepare_q21n_teacher_depth_audit as module


def write(path: Path, value: object) -> None:
    path.write_text(json.dumps(value), encoding="utf-8")


def main() -> None:
    categories = [
        f"{phase}/{material}"
        for phase in ("opening", "middlegame", "endgame")
        for material in ("stm_behind", "balanced", "stm_ahead")
        for _ in range(2)
    ]
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        corpus, shallow, forcing = root / "corpus.json", root / "shallow.json", root / "forcing.json"
        binary, weights, builder, summarizer = (
            root / "engine", root / "weights", root / "builder.py", root / "summarizer.py"
        )
        for path in (binary, weights, builder, summarizer):
            path.write_bytes(path.name.encode())
        positions = [{"id": f"p{i:02d}", "category": category} for i, category in enumerate(categories)]
        write(corpus, {
            "schema": "sekirei.q21i-ranking-corpus.v1",
            "diagnostic_only": True,
            "selection": {"score_blind": True},
            "positions": positions,
        })
        teacher = {
            "binary_sha256": module.sha256(binary),
            "weights_sha256": module.sha256(weights),
            "nnue_output": "residual-material",
        }
        write(shallow, {
            "schema": "sekirei.root-rank-teacher-corpus.v1",
            "diagnostic_only": True,
            "contract": {"depth": 3, "complete_legal_root_set": True},
            "source_corpus": {"sha256": module.sha256(corpus)},
            "teacher": teacher,
            "rows": [
                {"candidate_prefix_complete": True, "complete_legal_root_set": True}
                for _ in positions
            ],
        })
        entries = [
            {"id": position["id"], "forcing_class": "forcing_attack" if i == 0 else "quiet"}
            for i, position in enumerate(positions)
        ]
        write(forcing, {
            "schema": "sekirei.forcing-position-classification.v1",
            "diagnostic_only": True,
            "corpus": {"sha256": module.sha256(corpus)},
            "entries": entries,
        })
        document = module.prepare(argparse.Namespace(
            corpus=corpus, shallow=shallow, forcing=forcing, binary=binary,
            weights=weights, deep_builder=builder, summarizer=summarizer,
        ))
        assert document["status"] == "frozen_before_deep_labels"
        assert document["selection"]["positions"] == 18
        assert document["selection"]["balanced_positions"] == 6
        assert document["deep_contract"]["depth"] == 5
        assert document["decision_rule"]["q20_authorized"] is False
    print("test_prepare_q21n_teacher_depth_audit: PASS")


if __name__ == "__main__":
    main()
