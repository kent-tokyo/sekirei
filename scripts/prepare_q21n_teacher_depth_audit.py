#!/usr/bin/env python3
"""Freeze Q21n's shallow-versus-deep fixed-teacher ranking audit."""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter
from pathlib import Path
from typing import Any


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def bind(path: Path) -> dict[str, str]:
    if not path.is_file():
        raise ValueError(f"missing input: {path}")
    return {"path": str(path), "sha256": sha256(path)}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def prepare(args: argparse.Namespace) -> dict[str, Any]:
    corpus = json.loads(args.corpus.read_text(encoding="utf-8"))
    shallow = json.loads(args.shallow.read_text(encoding="utf-8"))
    forcing = json.loads(args.forcing.read_text(encoding="utf-8"))

    positions = corpus.get("positions", [])
    require(
        corpus.get("schema") == "sekirei.q21i-ranking-corpus.v1"
        and corpus.get("diagnostic_only") is True
        and corpus.get("selection", {}).get("score_blind") is True,
        "Q21n requires the frozen score-blind Q21i validation corpus",
    )
    require(len(positions) == 18, "Q21n requires all 18 frozen validation parents")
    categories = Counter(row.get("category") for row in positions)
    require(
        len(categories) == 9 and set(categories.values()) == {2},
        "Q21n corpus must contain two positions in every phase/material stratum",
    )
    require(
        sum(count for category, count in categories.items() if category.endswith("/balanced")) == 6,
        "Q21n corpus must retain six balanced positions",
    )

    shallow_rows = shallow.get("rows", [])
    require(
        shallow.get("schema") == "sekirei.root-rank-teacher-corpus.v1"
        and shallow.get("diagnostic_only") is True
        and shallow.get("contract", {}).get("depth") == 3
        and shallow.get("contract", {}).get("complete_legal_root_set") is True,
        "shallow teacher artifact is not a complete depth-3 root ranking",
    )
    require(
        shallow.get("source_corpus", {}).get("sha256") == sha256(args.corpus),
        "shallow teacher corpus hash differs from the frozen corpus",
    )
    require(
        len(shallow_rows) == 18
        and all(
            row.get("candidate_prefix_complete") is True
            and row.get("complete_legal_root_set") is True
            for row in shallow_rows
        ),
        "shallow teacher rows are incomplete",
    )

    forcing_entries = forcing.get("entries", [])
    require(
        forcing.get("schema") == "sekirei.forcing-position-classification.v1"
        and forcing.get("diagnostic_only") is True
        and forcing.get("corpus", {}).get("sha256") == sha256(args.corpus),
        "forcing classification is not bound to the frozen corpus",
    )
    require(
        {entry.get("id") for entry in forcing_entries}
        == {position.get("id") for position in positions},
        "forcing classification does not cover the exact 18 parents",
    )
    require(
        any(entry.get("forcing_class") != "quiet" for entry in forcing_entries),
        "Q21n corpus has no forcing position",
    )

    teacher = shallow.get("teacher", {})
    require(
        teacher.get("binary_sha256") == sha256(args.binary)
        and teacher.get("weights_sha256") == sha256(args.weights)
        and teacher.get("nnue_output") == "residual-material",
        "binary, weights, or output mode differs from the shallow teacher artifact",
    )

    return {
        "schema": "sekirei.q21n-teacher-depth-preregistration.v1",
        "status": "frozen_before_deep_labels",
        "diagnostic_only": True,
        "strength_claim": False,
        "question": (
            "Do complete depth-3 fixed-T root rankings remain decision-equivalent "
            "at complete depth 5 on the frozen validation parents?"
        ),
        "inputs": {
            "corpus": {**bind(args.corpus), "positions": 18},
            "shallow": {**bind(args.shallow), "depth": 3},
            "forcing": bind(args.forcing),
            "binary": bind(args.binary),
            "weights": {**bind(args.weights), "nnue_output": "residual-material"},
        },
        "selection": {
            "all_frozen_parents": True,
            "score_blind": True,
            "positions": 18,
            "balanced_positions": 6,
            "phase_material_counts": dict(sorted(categories.items())),
            "forcing_counts": dict(sorted(Counter(
                entry["forcing_class"] for entry in forcing_entries
            ).items())),
        },
        "deep_contract": {
            "depth": 5,
            "threads": 1,
            "spec_top_n": 0,
            "root_candidate_limit": 600,
            "complete_legal_root_set": True,
            "same_binary_weights_output_and_positions_as_shallow": True,
            "timeout_seconds_per_position": 300,
        },
        "metrics": {
            "tie_policy": "a shallow top-score tie succeeds if any tied move is deep-optimal",
            "ordinary_cp_limit": 10_000,
            "major_deep_regret_cp": 300,
            "moderate_deep_regret_cp": 100,
            "report": [
                "top-set agreement",
                "deep regret of best shallow-top move",
                "deep rank of best shallow-top move",
                "pairwise order agreement excluding score ties",
                "top-8 overlap",
            ],
            "strata": ["phase", "material_band", "forcing_class"],
        },
        "decision_rule": {
            "minimum_ordinary_positions": 12,
            "teacher_depth_insufficient_if": (
                "major deep regret >=300cp in at least 25% of ordinary positions"
            ),
            "mixed_if": (
                "major-regret rate is below 25% but moderate regret >=100cp occurs"
            ),
            "student_reproduction_primary_if": (
                "no ordinary position has deep regret >=100cp"
            ),
            "deep_T_is_ground_truth": False,
            "development_match_authorized": False,
            "q20_authorized": False,
        },
        "tools": {
            "preparer": bind(Path(__file__).resolve()),
            "deep_builder": bind(args.deep_builder),
            "summarizer": bind(args.summarizer),
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--shallow", type=Path, required=True)
    parser.add_argument("--forcing", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--weights", type=Path, required=True)
    parser.add_argument("--deep-builder", type=Path, required=True)
    parser.add_argument("--summarizer", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        document = prepare(args)
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps({"status": document["status"], **document["selection"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
