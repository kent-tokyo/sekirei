#!/usr/bin/env python3
"""Summarize train-only ranking updates at the quantized inference boundary."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def read(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def bind(path: Path) -> dict[str, str]:
    return {"path": str(path), "sha256": sha256(path)}


def metric(path: Path) -> dict[str, Any]:
    document = read(path)
    value = document.get("model_diagnostic")
    if document.get("schema") != "sekirei.root-rank-pair-audit.v1" or not isinstance(value, dict):
        raise ValueError(f"{path}: model diagnostic is required")
    return {
        "teacher_preferred_ordering_rate": value["teacher_preferred_ordering_rate"],
        "mean_pairwise_logistic_loss": value["mean_pairwise_logistic_loss"],
        "mean_parent_rank_loss_cp": value["mean_parent_rank_loss_cp"],
        "major_blunders_ge_300cp": value["major_blunders_ge_300cp"],
    }


def parse_candidate(value: str) -> tuple[str, Path, Path]:
    try:
        name, audit, metadata = value.split("=", 1)[0], *value.split("=", 1)[1].split(",", 1)
    except ValueError as error:
        raise argparse.ArgumentTypeError("candidate must be NAME=AUDIT,METADATA") from error
    return name, Path(audit), Path(metadata)


def summarize(args: argparse.Namespace) -> dict[str, Any]:
    baseline = metric(args.baseline)
    rows = []
    for name, audit_path, metadata_path in args.candidate:
        metadata = read(metadata_path)
        if metadata.get("schema") != "sekirei.ranking-training-run.v1":
            raise ValueError(f"{metadata_path}: ranking training metadata is required")
        current = metric(audit_path)
        rows.append({
            "name": name,
            "learning_rate": metadata["learning_rate"],
            "epochs": metadata["epochs"],
            **current,
            "rank_loss_reduction": (
                (baseline["mean_parent_rank_loss_cp"] - current["mean_parent_rank_loss_cp"])
                / baseline["mean_parent_rank_loss_cp"]
            ),
            "audit": bind(audit_path),
            "training_metadata": bind(metadata_path),
        })
    eligible = [
        row for row in rows
        if row["rank_loss_reduction"] >= 0.10
        and row["major_blunders_ge_300cp"] <= baseline["major_blunders_ge_300cp"]
    ]
    if not eligible:
        raise ValueError("no train-only recipe crosses the preregistered rank boundary")
    selected = min(
        eligible,
        key=lambda row: (
            row["mean_parent_rank_loss_cp"],
            row["mean_pairwise_logistic_loss"],
            -row["teacher_preferred_ordering_rate"],
            row["epochs"],
            row["learning_rate"],
        ),
    )
    return {
        "schema": "sekirei.q21q-transfer-boundary-diagnostic.v1",
        "status": "pass",
        "diagnostic_only": True,
        "strength_claim": False,
        "scope": "Q21p depth-7 training pairs only; no validation metric participates in selection",
        "baseline": {**baseline, "audit": bind(args.baseline)},
        "recipes": rows,
        "selected_recipe": {
            "name": selected["name"],
            "learning_rate": selected["learning_rate"],
            "epochs": selected["epochs"],
            "selection_used_validation": False,
            "selection_rule": (
                "among train rank-loss reductions >=10% with no major-blunder increase, "
                "minimize rank loss then logistic loss"
            ),
            "train_rank_loss_reduction": selected["rank_loss_reduction"],
        },
        "conclusion": (
            "The corrected objective reaches quantized inference, but the original 3-epoch "
            "1e-4 budget was too small to change parent choices."
        ),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--candidate", action="append", type=parse_candidate, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        document = summarize(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.write_text(
        json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps({"status": document["status"], "selected": document["selected_recipe"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
