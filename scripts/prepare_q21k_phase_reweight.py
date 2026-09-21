#!/usr/bin/env python3
"""Pre-register Q21k's one-factor phase-sampling content candidate."""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter
from pathlib import Path


PHASES = ("opening", "middlegame", "endgame")


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read_jsonl(path: Path) -> list[dict]:
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line]


def prepare(
    q21h_manifest_path: Path,
    q21i_preregistration_path: Path,
    teacher_cache: Path,
    trainer: Path,
    output: Path,
) -> dict:
    q21h = json.loads(q21h_manifest_path.read_text(encoding="utf-8"))
    q21i = json.loads(q21i_preregistration_path.read_text(encoding="utf-8"))
    if (
        q21h.get("schema") != "sekirei.q21h-independent-learning-split.v1"
        or q21h.get("selection_ready") is not True
    ):
        raise ValueError("Q21h split is not frozen and ready")
    if q21i.get("schema") != "sekirei.q21i-imitation-preregistration.v1":
        raise ValueError("unsupported Q21i control preregistration")
    train_path = Path(q21h["train"]["path"])
    validation_path = Path(q21h["validation"]["path"])
    if sha256(train_path) != q21h["train"]["sha256"]:
        raise ValueError("Q21h train hash mismatch")
    if sha256(validation_path) != q21h["validation"]["sha256"]:
        raise ValueError("Q21h validation hash mismatch")
    rows = read_jsonl(train_path)
    counts = Counter(row.get("tags", {}).get("phase") for row in rows)
    if set(counts) != set(PHASES) or sum(counts.values()) != q21h["train"]["positions"]:
        raise ValueError("Q21h train phases are incomplete")
    # Each phase contributes one third of the total effective sample mass.
    # The weighted mean remains exactly one, preserving the global LR scale.
    total = sum(counts.values())
    weights = {phase: total / (len(PHASES) * counts[phase]) for phase in PHASES}
    recipe = q21i["recipe_a"]
    candidate_a = Path("data/runs/q21i-imitation-pilot-20260921/recipe-a/candidate.best.bin")
    document = {
        "schema": "sekirei.q21k-phase-reweight-preregistration.v1",
        "status": "frozen_before_training",
        "diagnostic_only": True,
        "strength_claim": False,
        "single_factor": "phase sampling weights",
        "control": {
            "q21i_preregistration": {
                "path": str(q21i_preregistration_path),
                "sha256": sha256(q21i_preregistration_path),
            },
            "candidate_a_weights": {
                "path": str(candidate_a),
                "sha256": sha256(candidate_a),
            },
        },
        "fixed_inputs": {
            "q21h_manifest": {"path": str(q21h_manifest_path), "sha256": sha256(q21h_manifest_path)},
            "train": {"path": str(train_path), "sha256": sha256(train_path), "positions": len(rows)},
            "validation": {
                "path": str(validation_path),
                "sha256": sha256(validation_path),
                "positions": q21h["validation"]["positions"],
            },
            "teacher_and_initial_weights": q21i["inputs"]["teacher_and_initial_weights"],
            "teacher_cache": {"path": str(teacher_cache), "sha256": sha256(teacher_cache)},
            "trainer": {"path": str(trainer), "sha256": sha256(trainer)},
            "ranking_pairs": {
                "path": "data/runs/q21i-imitation-pilot-20260921/ranking-pairs.json",
                "sha256": sha256(Path("data/runs/q21i-imitation-pilot-20260921/ranking-pairs.json")),
            },
            "ranking_baseline": {
                "path": "data/runs/q21i-imitation-pilot-20260921/ranking-baseline.json",
                "sha256": sha256(Path("data/runs/q21i-imitation-pilot-20260921/ranking-baseline.json")),
            },
        },
        "fixed_recipe": recipe,
        "changed_factor": {
            "kind": "phase_sampling_weight",
            "unweighted_counts": dict(sorted(counts.items())),
            "weights": weights,
            "effective_mass": {phase: counts[phase] * weights[phase] for phase in PHASES},
            "weighted_mean": sum(counts[phase] * weights[phase] for phase in PHASES) / total,
        },
        "screen": {
            "frozen_validation_pairs": "Q21i ranking-pairs.json",
            "minimum_mean_parent_rank_loss_reduction": 0.10,
            "major_blunders_ge_300cp_must_not_increase": True,
            "development_match_only_after_screen_pass": True,
        },
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return document


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--q21h-manifest", type=Path, required=True)
    parser.add_argument("--q21i-preregistration", type=Path, required=True)
    parser.add_argument("--teacher-cache", type=Path, required=True)
    parser.add_argument("--trainer", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        document = prepare(
            args.q21h_manifest,
            args.q21i_preregistration,
            args.teacher_cache,
            args.trainer,
            args.output,
        )
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        parser.error(str(error))
    print(json.dumps(document["changed_factor"], sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
