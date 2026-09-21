#!/usr/bin/env python3
"""Pre-register Q21i deep-label imitation and its score-blind ranking screen."""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter
from pathlib import Path


STRATA = tuple(
    f"{phase}/{material}"
    for phase in ("opening", "middlegame", "endgame")
    for material in ("stm_behind", "balanced", "stm_ahead")
)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read_jsonl(path: Path) -> list[dict]:
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]


def stable_rank(row: dict, seed: int) -> str:
    source = row.get("source", {})
    identity = f"{source.get('source_key')}\0{row.get('sfen')}\0{seed}"
    return hashlib.sha256(identity.encode()).hexdigest()


def stratum(row: dict) -> str:
    tags = row.get("tags", {})
    value = f"{tags.get('phase')}/{tags.get('material_band')}"
    if value not in STRATA:
        raise ValueError(f"unsupported phase/material stratum: {value}")
    return value


def select_ranking_positions(rows: list[dict], per_stratum: int, seed: int) -> list[dict]:
    selected = []
    for wanted in STRATA:
        bucket = sorted((row for row in rows if stratum(row) == wanted), key=lambda row: stable_rank(row, seed))
        if len(bucket) < per_stratum:
            raise ValueError(f"{wanted} has {len(bucket)} rows; need {per_stratum}")
        selected.extend(bucket[:per_stratum])
    return selected


def prepare(
    q21h_manifest_path: Path,
    teacher_weights: Path,
    engine: Path,
    trainer: Path,
    auditor: Path,
    output_dir: Path,
    seed: int,
) -> dict:
    q21h = json.loads(q21h_manifest_path.read_text(encoding="utf-8"))
    if q21h.get("schema") != "sekirei.q21h-independent-learning-split.v1" or q21h.get("selection_ready") is not True:
        raise ValueError("Q21h manifest is not a ready independent split")
    train_path = Path(q21h["train"]["path"])
    validation_path = Path(q21h["validation"]["path"])
    if sha256(train_path) != q21h["train"]["sha256"] or sha256(validation_path) != q21h["validation"]["sha256"]:
        raise ValueError("Q21h split hash mismatch")
    rows = select_ranking_positions(read_jsonl(validation_path), 2, seed)
    positions = []
    for index, row in enumerate(rows, 1):
        positions.append({
            "id": f"q21i-rank-{index:02d}",
            "category": stratum(row),
            "initial_sfen": row["sfen"],
            "history_before_usi": [],
            "sfen": row["sfen"],
            "source": row["source"],
        })
    output_dir.mkdir(parents=True, exist_ok=True)
    ranking_corpus_path = output_dir / "ranking-corpus.json"
    ranking_corpus = {
        "schema": "sekirei.q21i-ranking-corpus.v1",
        "diagnostic_only": True,
        "strength_claim": False,
        "selection": {
            "score_blind": True,
            "seed": seed,
            "per_phase_material_stratum": 2,
            "positions": len(positions),
            "counts": dict(sorted(Counter(position["category"] for position in positions).items())),
        },
        "positions": positions,
    }
    ranking_corpus_path.write_text(json.dumps(ranking_corpus, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    preregistration = {
        "schema": "sekirei.q21i-imitation-preregistration.v1",
        "status": "planned",
        "strength_claim": False,
        "single_factor": "fine-tune T from a fresh optimizer on fixed-T 100k-node search labels",
        "inputs": {
            "q21h_manifest": {"path": str(q21h_manifest_path), "sha256": sha256(q21h_manifest_path)},
            "train": {"path": str(train_path), "sha256": sha256(train_path), "positions": q21h["train"]["positions"]},
            "validation": {"path": str(validation_path), "sha256": sha256(validation_path), "positions": q21h["validation"]["positions"]},
            "teacher_and_initial_weights": {"path": str(teacher_weights), "sha256": sha256(teacher_weights), "nnue_output": "residual-material"},
            "ranking_corpus": {"path": str(ranking_corpus_path), "sha256": sha256(ranking_corpus_path), "positions": len(positions)},
        },
        "binaries": {
            "engine": {"path": str(engine), "sha256": sha256(engine)},
            "trainer": {"path": str(trainer), "sha256": sha256(trainer)},
            "ranking_auditor": {"path": str(auditor), "sha256": sha256(auditor)},
        },
        "recipe_a": {
            "seed": seed,
            "epochs": 3,
            "fresh_optimizer": True,
            "init_weights": "teacher_and_initial_weights",
            "teacher_eval": "nnue",
            "teacher_nnue_output": "residual-material",
            "student_nnue_output": "residual-material",
            "label_nodes": 100000,
            "teacher_score_cap_cp": 3000,
            "exclude_mate_labels": True,
            "learning_rate": 0.0001,
            "schedule": "constant",
            "shuffle_seed": 4242,
        },
        "recipe_b_contingency": {
            "allowed_only_if_recipe_a_fails_ranking_screen": True,
            "only_change": "learning_rate",
            "learning_rate": 0.00003,
            "maximum_total_recipes": 2,
        },
        "ranking_screen": {
            "teacher_root_depth": 3,
            "complete_legal_root_set": True,
            "normal_cp_only": True,
            "pass": "mean parent rank loss improves by at least 10% and major blunders >=300cp do not increase",
            "strength_claim": False,
        },
        "development_match_screen": {
            "conditional_on_ranking_pass": True,
            "new_start_positions": 16,
            "color_reversed_games": 2,
            "byoyomi_ms": 1000,
            "maximum_games": 32,
            "baseline": "material",
            "pass_score": 0.60,
            "reject_below": 0.40,
        },
    }
    path = output_dir / "preregistration.json"
    path.write_text(json.dumps(preregistration, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return preregistration


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--q21h-manifest", type=Path, required=True)
    parser.add_argument("--teacher-weights", type=Path, required=True)
    parser.add_argument("--engine", type=Path, required=True)
    parser.add_argument("--trainer", type=Path, required=True)
    parser.add_argument("--auditor", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--seed", type=int, default=42)
    args = parser.parse_args()
    document = prepare(args.q21h_manifest, args.teacher_weights, args.engine, args.trainer, args.auditor, args.output_dir, args.seed)
    print(json.dumps({"status": document["status"], "ranking_positions": document["inputs"]["ranking_corpus"]["positions"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
