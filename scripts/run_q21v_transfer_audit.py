#!/usr/bin/env python3
"""Run Q21v teacher-distribution, activation, and LOPO transfer diagnostics."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import statistics
import subprocess
from collections import Counter
from pathlib import Path
from typing import Any


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def bind(path: Path) -> dict[str, str]:
    if not path.is_file():
        raise ValueError(f"missing artifact: {path}")
    return {"path": str(path), "sha256": sha256(path)}


def read(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def atomic_write(path: Path, document: dict[str, Any]) -> None:
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(
        json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    os.replace(temporary, path)


def run_json(command: list[str], timeout: float) -> dict[str, Any]:
    completed = subprocess.run(
        command,
        text=True,
        capture_output=True,
        check=False,
        timeout=timeout,
        env={**os.environ, "RAYON_NUM_THREADS": "1"},
    )
    if completed.returncode:
        raise RuntimeError(
            f"command failed ({completed.returncode}): {' '.join(command)}\n{completed.stderr[-4000:]}"
        )
    return json.loads(completed.stdout)


def audit(auditor: Path, pairs: Path, weights: Path) -> dict[str, Any]:
    return run_json([str(auditor), "--pairs", str(pairs), "--weights", str(weights)], 120)


def metrics(audit_document: dict[str, Any]) -> dict[str, Any]:
    model = audit_document["model_diagnostic"]
    parents = model["parent_diagnostics"]
    return {
        "parents": len(parents),
        "mean_direct_top_regret_cp": model["mean_parent_rank_loss_cp"],
        "top1_matches": sum(row["teacher_rank_loss_cp"] == 0 for row in parents),
        "major_regrets_ge_300cp": sum(row["teacher_rank_loss_cp"] >= 300 for row in parents),
        "ordered_pair_rate": model["teacher_preferred_ordering_rate"],
    }


def relative_reduction(baseline: float, candidate: float) -> float:
    return (baseline - candidate) / baseline if baseline > 0 else 0.0


def teacher_distribution(corpus: dict[str, Any]) -> dict[str, Any]:
    by_parent: dict[str, dict[str, int]] = {}
    categories: dict[str, str] = {}
    for pair in corpus["pairs"]:
        parent = pair["parent_id"]
        categories[parent] = pair["category"]
        losses = by_parent.setdefault(parent, {})
        losses.setdefault(pair["higher_move_usi"], 0)
        lower = pair["lower_move_usi"]
        losses[lower] = max(losses.get(lower, 0), pair["teacher_score_gap_cp"])
    gaps = [loss for losses in by_parent.values() for loss in losses.values() if loss > 0]
    candidate_counts = [len(losses) for losses in by_parent.values()]
    return {
        "parents": len(by_parent),
        "category_parents": dict(sorted(Counter(categories.values()).items())),
        "candidate_count": {
            "minimum": min(candidate_counts),
            "maximum": max(candidate_counts),
            "mean": statistics.mean(candidate_counts),
            "median": statistics.median(candidate_counts),
        },
        "nonzero_teacher_gap_cp": {
            "count": len(gaps),
            "minimum": min(gaps),
            "maximum": max(gaps),
            "mean": statistics.mean(gaps),
            "median": statistics.median(gaps),
        },
    }


def score_transfer(baseline: dict[str, Any], candidate: dict[str, Any]) -> dict[str, Any]:
    def rows(document: dict[str, Any]) -> dict[tuple[str, str], dict[str, Any]]:
        return {
            (row["parent_id"], row["move_usi"]): row
            for row in document["model_diagnostic"]["move_diagnostics"]
        }

    before = rows(baseline)
    after = rows(candidate)
    require(set(before) == set(after), "baseline/candidate move diagnostic set differs")
    changes = [after[key]["parent_score_cp"] - before[key]["parent_score_cp"] for key in before]
    absolute = [abs(value) for value in changes]
    top_absolute = [
        abs(after[key]["parent_score_cp"] - before[key]["parent_score_cp"])
        for key in before
        if before[key]["teacher_top"]
    ]
    rest_absolute = [
        abs(after[key]["parent_score_cp"] - before[key]["parent_score_cp"])
        for key in before
        if not before[key]["teacher_top"]
    ]
    return {
        "moves": len(changes),
        "changed_moves": sum(value != 0 for value in changes),
        "mean_signed_score_delta_cp": statistics.mean(changes),
        "mean_abs_score_delta_cp": statistics.mean(absolute),
        "median_abs_score_delta_cp": statistics.median(absolute),
        "maximum_abs_score_delta_cp": max(absolute),
        "teacher_top_mean_abs_delta_cp": statistics.mean(top_absolute),
        "rest_mean_abs_delta_cp": statistics.mean(rest_absolute),
    }


def write_fold_corpus(source: dict[str, Any], pairs: list[dict[str, Any]], path: Path) -> None:
    document = {**source, "pairs": pairs}
    atomic_write(path, document)


def train_fold(
    trainer: Path,
    train_pairs: Path,
    initial_weights: Path,
    output: Path,
    contract: dict[str, Any],
) -> None:
    metadata = output.with_suffix(".ranking.json")
    if output.is_file() and metadata.is_file():
        existing = read(metadata)
        require(
            existing.get("pairs_path") == str(train_pairs)
            and existing.get("epochs") == contract["epochs"]
            and existing.get("ranking_objective") == contract["ranking_objective"],
            f"existing LOPO candidate contract mismatch: {output}",
        )
        return
    completed = subprocess.run(
        [
            str(trainer),
            "--ranking-pairs", str(train_pairs),
            "--ranking-objective", contract["ranking_objective"],
            "--ranking-temperature-cp", str(contract["temperature_cp"]),
            "--ranking-parent-balanced",
            "--ranking-batch-pairs", "1",
            "--ranking-epochs", str(contract["epochs"]),
            "--init-weights", str(initial_weights),
            "--nnue-output", contract["nnue_output"],
            "--lr", str(contract["learning_rate"]),
            "--seed", str(contract["seed"]),
            "--output", str(output),
        ],
        text=True,
        capture_output=True,
        check=False,
        timeout=900,
        env={**os.environ, "RAYON_NUM_THREADS": "1"},
    )
    if completed.returncode:
        raise RuntimeError(f"LOPO training failed: {completed.stderr[-4000:]}")


def run(args: argparse.Namespace) -> dict[str, Any]:
    prereg = read(args.preregistration)
    require(
        prereg.get("schema") == "sekirei.q21v-transfer-audit-preregistration.v1"
        and prereg.get("status") == "frozen_before_lopo_training",
        "unexpected Q21v preregistration",
    )
    for name, path in (
        ("train_pairs", args.train_pairs),
        ("validation_pairs", args.validation_pairs),
        ("initial_weights", args.initial_weights),
        ("candidate", args.candidate),
        ("trainer", args.trainer),
        ("ranking_auditor", args.ranking_auditor),
    ):
        require(prereg["inputs"][name]["sha256"] == sha256(path), f"{name} SHA mismatch")
    require(
        prereg["inputs"]["runner"]["sha256"] == sha256(Path(__file__).resolve()),
        "Q21v runner changed after preregistration",
    )
    args.output_dir.mkdir(parents=True, exist_ok=True)
    train = read(args.train_pairs)
    validation = read(args.validation_pairs)

    audits: dict[str, dict[str, Any]] = {}
    for corpus_name, corpus_path in (("train", args.train_pairs), ("validation", args.validation_pairs)):
        for arm, weights in (("baseline", args.initial_weights), ("candidate", args.candidate)):
            document = audit(args.ranking_auditor, corpus_path, weights)
            path = args.output_dir / f"{corpus_name}-{arm}-audit.json"
            atomic_write(path, document)
            audits[f"{corpus_name}_{arm}"] = document

    contract = prereg["contract"]
    parent_ids = sorted({pair["parent_id"] for pair in train["pairs"]})
    require(len(parent_ids) == contract["lopo_folds"], "LOPO parent count mismatch")
    lopo_rows = []
    lopo_dir = args.output_dir / "lopo"
    lopo_dir.mkdir(parents=True, exist_ok=True)
    for fold, heldout in enumerate(parent_ids, 1):
        fold_dir = lopo_dir / f"fold-{fold:02d}"
        fold_dir.mkdir(parents=True, exist_ok=True)
        training_rows = [pair for pair in train["pairs"] if pair["parent_id"] != heldout]
        heldout_rows = [pair for pair in train["pairs"] if pair["parent_id"] == heldout]
        require(training_rows and heldout_rows, f"{heldout}: empty LOPO partition")
        train_path = fold_dir / "train-pairs.json"
        heldout_path = fold_dir / "heldout-pairs.json"
        write_fold_corpus(train, training_rows, train_path)
        write_fold_corpus(train, heldout_rows, heldout_path)
        candidate_path = fold_dir / "candidate.bin"
        train_fold(args.trainer, train_path, args.initial_weights, candidate_path, contract)
        baseline_audit = audit(args.ranking_auditor, heldout_path, args.initial_weights)
        candidate_audit = audit(args.ranking_auditor, heldout_path, candidate_path)
        baseline_path = fold_dir / "baseline-audit.json"
        candidate_audit_path = fold_dir / "candidate-audit.json"
        atomic_write(baseline_path, baseline_audit)
        atomic_write(candidate_audit_path, candidate_audit)
        lopo_rows.append(
            {
                "fold": fold,
                "heldout_parent": heldout,
                "train_pairs": bind(train_path),
                "heldout_pairs": bind(heldout_path),
                "candidate": bind(candidate_path),
                "training": bind(candidate_path.with_suffix(".ranking.json")),
                "baseline_audit": bind(baseline_path),
                "candidate_audit": bind(candidate_audit_path),
                "baseline": metrics(baseline_audit),
                "candidate_metrics": metrics(candidate_audit),
            }
        )
        print(f"q21v LOPO: {fold}/{len(parent_ids)} parents", flush=True)

    lopo_baseline_regret = statistics.mean(
        row["baseline"]["mean_direct_top_regret_cp"] for row in lopo_rows
    )
    lopo_candidate_regret = statistics.mean(
        row["candidate_metrics"]["mean_direct_top_regret_cp"] for row in lopo_rows
    )
    lopo = {
        "folds": len(lopo_rows),
        "baseline_mean_direct_top_regret_cp": lopo_baseline_regret,
        "candidate_mean_direct_top_regret_cp": lopo_candidate_regret,
        "direct_top_regret_reduction": relative_reduction(
            lopo_baseline_regret, lopo_candidate_regret
        ),
        "baseline_top1_matches": sum(row["baseline"]["top1_matches"] for row in lopo_rows),
        "candidate_top1_matches": sum(
            row["candidate_metrics"]["top1_matches"] for row in lopo_rows
        ),
        "baseline_major_regrets_ge_300cp": sum(
            row["baseline"]["major_regrets_ge_300cp"] for row in lopo_rows
        ),
        "candidate_major_regrets_ge_300cp": sum(
            row["candidate_metrics"]["major_regrets_ge_300cp"] for row in lopo_rows
        ),
        "rows": lopo_rows,
    }

    train_baseline = metrics(audits["train_baseline"])
    train_candidate = metrics(audits["train_candidate"])
    validation_baseline = metrics(audits["validation_baseline"])
    validation_candidate = metrics(audits["validation_candidate"])
    train_distribution = teacher_distribution(train)
    validation_distribution = teacher_distribution(validation)
    candidate_count_relative_difference = abs(
        validation_distribution["candidate_count"]["mean"]
        - train_distribution["candidate_count"]["mean"]
    ) / train_distribution["candidate_count"]["mean"]
    gap_relative_difference = abs(
        validation_distribution["nonzero_teacher_gap_cp"]["median"]
        - train_distribution["nonzero_teacher_gap_cp"]["median"]
    ) / train_distribution["nonzero_teacher_gap_cp"]["median"]
    activation_rows = {
        name: document["model_diagnostic"]["mean_activation"]
        for name, document in audits.items()
    }
    activation_collapse = any(
        row["ft_active_ratio"] < 0.10
        or row["ft_saturated_ratio"] > 0.50
        or row["l2_active_ratio"] < 0.10
        or row["l2_saturated_ratio"] > 0.50
        for row in activation_rows.values()
    )
    activation_comparison = {
        "baseline_ft_active_ratio_absolute_difference": abs(
            activation_rows["train_baseline"]["ft_active_ratio"]
            - activation_rows["validation_baseline"]["ft_active_ratio"]
        ),
        "candidate_ft_active_ratio_absolute_difference": abs(
            activation_rows["train_candidate"]["ft_active_ratio"]
            - activation_rows["validation_candidate"]["ft_active_ratio"]
        ),
        "baseline_l2_active_ratio_absolute_difference": abs(
            activation_rows["train_baseline"]["l2_active_ratio"]
            - activation_rows["validation_baseline"]["l2_active_ratio"]
        ),
        "candidate_l2_active_ratio_absolute_difference": abs(
            activation_rows["train_candidate"]["l2_active_ratio"]
            - activation_rows["validation_candidate"]["l2_active_ratio"]
        ),
    }
    train_score_transfer = score_transfer(audits["train_baseline"], audits["train_candidate"])
    validation_score_transfer = score_transfer(
        audits["validation_baseline"], audits["validation_candidate"]
    )
    train_reduction = relative_reduction(
        train_baseline["mean_direct_top_regret_cp"],
        train_candidate["mean_direct_top_regret_cp"],
    )
    validation_reduction = relative_reduction(
        validation_baseline["mean_direct_top_regret_cp"],
        validation_candidate["mean_direct_top_regret_cp"],
    )
    overfit_signal = train_reduction >= 0.10 and lopo["direct_top_regret_reduction"] < 0.05
    distribution_shift_signal = (
        candidate_count_relative_difference >= 0.25 or gap_relative_difference >= 0.25
    )
    signals = {
        "parent_memorization_or_sample_scarcity": overfit_signal,
        "teacher_distribution_shift": distribution_shift_signal,
        "hidden_activation_collapse": activation_collapse,
        "architecture_capacity_isolated": False,
        "quantization_blocks_all_score_movement": (
            train_score_transfer["changed_moves"] == 0
            and validation_score_transfer["changed_moves"] == 0
        ),
    }
    if overfit_signal:
        primary = "parent_memorization_or_sample_scarcity"
    elif activation_collapse:
        primary = "hidden_activation_collapse"
    elif distribution_shift_signal:
        primary = "teacher_distribution_shift"
    else:
        primary = "unresolved_without_a_single-factor_follow-up"
    document = {
        "schema": "sekirei.q21v-transfer-audit-decision.v1",
        "status": "complete",
        "diagnostic_only": True,
        "strength_claim": False,
        "candidate_adopted": False,
        "development_match_authorized": False,
        "q20_authorized": False,
        "preregistration": bind(args.preregistration),
        "train": {
            "baseline": train_baseline,
            "candidate": train_candidate,
            "direct_top_regret_reduction": train_reduction,
            "teacher_distribution": train_distribution,
            "score_transfer": train_score_transfer,
        },
        "validation": {
            "baseline": validation_baseline,
            "candidate": validation_candidate,
            "direct_top_regret_reduction": validation_reduction,
            "teacher_distribution": validation_distribution,
            "score_transfer": validation_score_transfer,
        },
        "distribution_comparison": {
            "candidate_count_mean_relative_difference": candidate_count_relative_difference,
            "teacher_gap_median_relative_difference": gap_relative_difference,
        },
        "activation": activation_rows,
        "activation_comparison": activation_comparison,
        "lopo": lopo,
        "signals": signals,
        "primary_diagnosis": primary,
        "capacity_conclusion": (
            "not isolated: Q21v does not change architecture width; increase capacity only after "
            "the LOPO and distribution evidence no longer explains the transfer failure"
        ),
        "next_action": (
            "increase independent train-parent coverage before another candidate; preserve Q21u "
            "validation as audit-only and require a new fresh holdout"
            if overfit_signal
            else "run one preregistered single-factor diagnostic matching the primary signal"
        ),
        "artifacts": {
            "train_baseline_audit": bind(args.output_dir / "train-baseline-audit.json"),
            "train_candidate_audit": bind(args.output_dir / "train-candidate-audit.json"),
            "validation_baseline_audit": bind(args.output_dir / "validation-baseline-audit.json"),
            "validation_candidate_audit": bind(args.output_dir / "validation-candidate-audit.json"),
            "runner": bind(Path(__file__).resolve()),
        },
    }
    atomic_write(args.output_dir / "decision.json", document)
    report = [
        "# Q21v transfer audit",
        "",
        "This is a diagnostic transfer audit, not a strength result.",
        "",
        f"- Primary diagnosis: `{primary}`.",
        f"- Full-train regret reduction: {train_reduction * 100:.2f}%.",
        f"- LOPO regret reduction: {lopo['direct_top_regret_reduction'] * 100:.2f}%.",
        f"- Fresh holdout regret reduction: {validation_reduction * 100:.2f}%.",
        f"- Distribution-shift signal: {distribution_shift_signal}.",
        f"- Hidden-activation-collapse signal: {activation_collapse}.",
        "- Architecture capacity was not changed and is not isolated.",
        "- Candidate remains rejected; development match and Q20 remain unauthorized.",
        "",
    ]
    (args.output_dir / "report.md").write_text("\n".join(report), encoding="utf-8")
    return document


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preregistration", type=Path, required=True)
    parser.add_argument("--train-pairs", type=Path, required=True)
    parser.add_argument("--validation-pairs", type=Path, required=True)
    parser.add_argument("--initial-weights", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--trainer", type=Path, required=True)
    parser.add_argument("--ranking-auditor", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    try:
        document = run(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError, RuntimeError) as error:
        parser.error(str(error))
    print(
        json.dumps(
            {
                "status": document["status"],
                "primary_diagnosis": document["primary_diagnosis"],
                "lopo_reduction": document["lopo"]["direct_top_regret_reduction"],
            },
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
