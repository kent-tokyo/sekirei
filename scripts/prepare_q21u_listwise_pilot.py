#!/usr/bin/env python3
"""Calibrate and freeze Q21u's train-only listwise recipe search."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
from pathlib import Path
from typing import Any


TEMPERATURES_CP = (25, 50, 75, 100, 150, 200, 300, 400, 600, 800, 1200)
RECIPES = (
    {"id": "e15-lr0003", "epochs": 15, "learning_rate": 0.0003},
    {"id": "e30-lr0003", "epochs": 30, "learning_rate": 0.0003},
    {"id": "e30-lr0010", "epochs": 30, "learning_rate": 0.0010},
    {"id": "e60-lr0003", "epochs": 60, "learning_rate": 0.0003},
    {"id": "e60-lr0010", "epochs": 60, "learning_rate": 0.0010},
)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def bind(path: Path) -> dict[str, str]:
    if not path.is_file():
        raise ValueError(f"missing input: {path}")
    return {"path": str(path), "sha256": sha256(path)}


def read(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def softmax(scores: dict[str, int], temperature: float) -> dict[str, float]:
    maximum = max(scores.values()) / temperature
    values = {move: math.exp(score / temperature - maximum) for move, score in scores.items()}
    total = sum(values.values())
    return {move: value / total for move, value in values.items()}


def calibration(
    shallow: dict[str, Any], measurements: dict[str, Any]
) -> tuple[list[dict[str, Any]], int]:
    shallow_by_id = {
        row["id"]: {
            candidate["move"]: candidate["score_cp"]
            for candidate in row["teacher_root"]["root_candidates"]
            if isinstance(candidate.get("score_cp"), int)
            and abs(candidate["score_cp"]) <= 10_000
        }
        for row in shallow["rows"]
    }
    rows = []
    for temperature in TEMPERATURES_CP:
        losses = []
        for row in measurements["rows"]:
            fixed = row["fixed"]
            require(len(fixed) == 2, f"{row['id']}: expected two depth-7 repeats")
            deep = {
                move: fixed[0][move]["score_cp"]
                for move in row["candidate_moves"]
                if move in shallow_by_id[row["id"]]
                and isinstance(fixed[0][move].get("score_cp"), int)
                and abs(fixed[0][move]["score_cp"]) <= 10_000
                and fixed[0][move]["score_cp"] == fixed[1][move]["score_cp"]
            }
            shallow_scores = {move: shallow_by_id[row["id"]][move] for move in deep}
            require(len(shallow_scores) >= 2, f"{row['id']}: insufficient overlapping scores")
            best = max(deep.values())
            teacher_tops = [move for move, score in deep.items() if score == best]
            probability = softmax(shallow_scores, temperature)
            losses.append(
                -sum(math.log(probability[move]) for move in teacher_tops) / len(teacher_tops)
            )
        rows.append(
            {
                "temperature_cp": temperature,
                "parents": len(losses),
                "mean_depth7_top_nll_from_depth3_distribution": sum(losses) / len(losses),
            }
        )
    selected = min(
        rows,
        key=lambda row: (
            row["mean_depth7_top_nll_from_depth3_distribution"],
            row["temperature_cp"],
        ),
    )["temperature_cp"]
    return rows, selected


def prepare(args: argparse.Namespace) -> dict[str, Any]:
    shallow = read(args.shallow_teacher)
    measurements = read(args.depth7_measurements)
    pairs = read(args.train_pairs)
    require(
        shallow.get("schema") == "sekirei.root-rank-teacher-corpus.v1"
        and len(shallow.get("rows", [])) == 18,
        "unexpected shallow teacher corpus",
    )
    require(
        measurements.get("schema") == "sekirei.q21p-depth7-label-measurements.v1"
        and len(measurements.get("rows", [])) == 18,
        "unexpected depth-7 measurement corpus",
    )
    require(
        pairs.get("schema") == "sekirei.root-rank-pairs.v1"
        and pairs.get("pair_selection") == "top-vs-rest"
        and len({row["parent_id"] for row in pairs.get("pairs", [])}) == 18,
        "Q21u requires 18-parent direct top-versus-rest train pairs",
    )
    calibration_rows, selected_temperature = calibration(shallow, measurements)
    document = {
        "schema": "sekirei.q21u-listwise-train-preregistration.v1",
        "status": "frozen_before_any_q21u_training_or_validation_selection",
        "diagnostic_only": True,
        "strength_claim": False,
        "single_factor": (
            "replace sign-only pairwise logistic ranking with parent-level gap-aware listwise softmax"
        ),
        "temperature_calibration": {
            "selection_data": "train parents only; no validation or match positions",
            "criterion": "minimum NLL assigned by depth-3 distribution to the depth-7 top move set",
            "grid_cp": list(TEMPERATURES_CP),
            "rows": calibration_rows,
            "selected_temperature_cp": selected_temperature,
        },
        "recipe_search": {
            "selection_data": "same 18 train parents only",
            "seed": 42,
            "fresh_optimizer": True,
            "nnue_output": "residual-material",
            "parent_balanced": True,
            "recipes": list(RECIPES),
            "selection_order": [
                "fewest train major regrets >=300cp",
                "most quantized train top-1 matches",
                "lowest train mean direct top regret",
                "lowest final listwise cross-entropy",
                "recipe id",
            ],
            "minimum_progress": (
                "chosen recipe must change quantized parameters and improve either top-1 matches "
                "or mean direct top regret without increasing major regrets"
            ),
        },
        "validation_boundary": (
            "do not select or label Q21u validation positions until recipe-decision.json is frozen"
        ),
        "inputs": {
            "shallow_teacher": bind(args.shallow_teacher),
            "depth7_measurements": bind(args.depth7_measurements),
            "train_pairs": bind(args.train_pairs),
            "initial_weights": bind(args.initial_weights),
            "trainer": bind(args.trainer),
            "ranking_auditor": bind(args.ranking_auditor),
            "preparer": bind(Path(__file__).resolve()),
        },
    }
    args.output_dir.mkdir(parents=True, exist_ok=True)
    output = args.output_dir / "preregistration.json"
    encoded = json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n"
    if output.exists() and output.read_text(encoding="utf-8") != encoded:
        raise ValueError("existing Q21u preregistration differs")
    output.write_text(encoded, encoding="utf-8")
    return document


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--shallow-teacher", type=Path, required=True)
    parser.add_argument("--depth7-measurements", type=Path, required=True)
    parser.add_argument("--train-pairs", type=Path, required=True)
    parser.add_argument("--initial-weights", type=Path, required=True)
    parser.add_argument("--trainer", type=Path, required=True)
    parser.add_argument("--ranking-auditor", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    try:
        document = prepare(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    print(json.dumps({
        "status": document["status"],
        "selected_temperature_cp": document["temperature_calibration"]["selected_temperature_cp"],
        "recipes": len(document["recipe_search"]["recipes"]),
    }, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
