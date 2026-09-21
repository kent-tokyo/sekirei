#!/usr/bin/env python3
"""Freeze Q21m's one-factor pairwise-ranking pilot before labels or training."""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter
from pathlib import Path
from typing import Any


PHASES = ("opening", "middlegame", "endgame")
MATERIAL_BANDS = ("stm_behind", "balanced", "stm_ahead")
STRATA = tuple(f"{phase}/{material}" for phase in PHASES for material in MATERIAL_BANDS)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read_jsonl(path: Path) -> list[dict[str, Any]]:
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line]


def stratum(row: dict[str, Any]) -> str:
    tags = row.get("tags", {})
    value = f"{tags.get('phase')}/{tags.get('material_band')}"
    if value not in STRATA:
        raise ValueError(f"unsupported phase/material stratum: {value}")
    return value


def rank(row: dict[str, Any], seed: int) -> str:
    source = row.get("source", {})
    identity = f"{seed}\0{source.get('source_key')}\0{row.get('sfen')}"
    return hashlib.sha256(identity.encode()).hexdigest()


def select(rows: list[dict[str, Any]], per_stratum: int, seed: int) -> list[dict[str, Any]]:
    selected = []
    for wanted in STRATA:
        bucket = sorted((row for row in rows if stratum(row) == wanted), key=lambda row: rank(row, seed))
        if len(bucket) < per_stratum:
            raise ValueError(f"{wanted} has {len(bucket)} rows; need {per_stratum}")
        selected.extend(bucket[:per_stratum])
    return selected


def bind(path: Path) -> dict[str, Any]:
    if not path.is_file():
        raise ValueError(f"missing input: {path}")
    return {"path": str(path), "sha256": sha256(path)}


def prepare(args: argparse.Namespace) -> tuple[dict[str, Any], dict[str, Any]]:
    q21h = json.loads(args.q21h_manifest.read_text(encoding="utf-8"))
    if (
        q21h.get("schema") != "sekirei.q21h-independent-learning-split.v1"
        or q21h.get("selection_ready") is not True
    ):
        raise ValueError("Q21h split is not frozen and ready")
    train_path = Path(q21h["train"]["path"])
    if sha256(train_path) != q21h["train"]["sha256"]:
        raise ValueError("Q21h train SHA mismatch")
    selected = select(read_jsonl(train_path), args.per_stratum, args.selection_seed)
    positions = [
        {
            "id": f"q21m-train-{index:02d}",
            "category": stratum(row),
            "initial_sfen": row["sfen"],
            "history_before_usi": [],
            "sfen": row["sfen"],
            "source": row["source"],
        }
        for index, row in enumerate(selected, 1)
    ]
    corpus = {
        "schema": "sekirei.q21m-ranking-train-corpus.v1",
        "diagnostic_only": True,
        "strength_claim": False,
        "selection": {
            "score_blind": True,
            "source_arm": "Q21h train only",
            "seed": args.selection_seed,
            "per_phase_material_stratum": args.per_stratum,
            "positions": len(positions),
            "counts": dict(sorted(Counter(item["category"] for item in positions).items())),
        },
        "positions": positions,
    }
    args.output_dir.mkdir(parents=True, exist_ok=True)
    corpus_path = args.output_dir / "train-ranking-corpus.json"
    corpus_path.write_text(json.dumps(corpus, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    preregistration = {
        "schema": "sekirei.q21m-pairwise-pilot-preregistration.v1",
        "status": "frozen_before_teacher_labels",
        "diagnostic_only": True,
        "strength_claim": False,
        "hypothesis": (
            "A parent-balanced pairwise objective on fixed-T root rankings transfers legal-move "
            "ordering better than another scalar-MSE or sampling-only update."
        ),
        "single_factor": "training target/objective: scalar position MSE -> parent-balanced pairwise legal-move ranking",
        "fixed": {
            "initial_weights": bind(args.teacher),
            "teacher_weights": bind(args.teacher),
            "teacher_output": "residual-material",
            "source_split": bind(args.q21h_manifest),
            "train_source": bind(train_path),
            "init_seed": 42,
            "fresh_optimizer": True,
            "learning_rate": 0.0001,
        },
        "evidence": {
            "q21l_strata": bind(args.q21l_strata),
            "reason": (
                "Q21k candidate/material bestmoves disagreed on all 8 balanced positions; "
                "the candidate had more major actual-move regret than material, while shallow-depth "
                "positions did not contain the major-regret events."
            ),
        },
        "train_corpus": {**bind(corpus_path), "positions": len(positions)},
        "teacher_label_contract": {
            "depth": 3,
            "threads": 1,
            "spec_top_n": 0,
            "complete_legal_root_set": True,
            "root_candidate_limit": 600,
            "normal_cp_only": True,
            "pair_selection": "adjacent",
            "top_k": 8,
        },
        "training_contract": {
            "epochs": 3,
            "ranking_parent_balanced": True,
            "ranking_batch_pairs": 1,
            "nnue_output": "residual-material",
            "no_scalar_targets": True,
        },
        "validation_contract": {
            "pairs": bind(args.validation_pairs),
            "baseline_audit": bind(args.baseline_audit),
            "minimum_mean_parent_rank_loss_reduction": 0.10,
            "major_blunders_ge_300cp_must_not_increase": True,
            "development_match_only_after_screen_pass": True,
        },
        "development_match_contract": {
            "fresh_openings": 16,
            "games_per_position": 2,
            "byoyomi_ms": 1000,
            "baseline": "material",
            "reject_below": 0.40,
            "pass_at_or_above": 0.60,
            "middle_band": "one additional fresh 16-opening batch only",
        },
        "binaries": {
            "engine": bind(args.engine),
            "trainer": bind(args.trainer),
            "ranking_auditor": bind(args.auditor),
        },
        "sources": {
            "preparer": bind(Path(__file__).resolve()),
            "teacher_builder": bind(args.teacher_builder),
            "pair_builder": bind(args.pair_builder),
        },
    }
    prereg_path = args.output_dir / "preregistration.json"
    prereg_path.write_text(
        json.dumps(preregistration, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return preregistration, corpus


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--q21h-manifest", type=Path, required=True)
    parser.add_argument("--q21l-strata", type=Path, required=True)
    parser.add_argument("--teacher", type=Path, required=True)
    parser.add_argument("--validation-pairs", type=Path, required=True)
    parser.add_argument("--baseline-audit", type=Path, required=True)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--trainer", type=Path, required=True)
    parser.add_argument("--auditor", type=Path, required=True)
    parser.add_argument("--teacher-builder", type=Path, required=True)
    parser.add_argument("--pair-builder", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--selection-seed", type=int, default=4343)
    parser.add_argument("--per-stratum", type=int, default=2)
    args = parser.parse_args()
    try:
        preregistration, corpus = prepare(args)
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        parser.error(str(error))
    print(json.dumps({
        "status": preregistration["status"],
        "positions": corpus["selection"]["positions"],
        "single_factor": preregistration["single_factor"],
    }, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
