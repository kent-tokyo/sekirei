#!/usr/bin/env python3
"""Freeze Q21p's train-only recipe retry and a fresh validation hold-out."""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter
from pathlib import Path
from typing import Any


STRATA = tuple(
    f"{phase}/{material}"
    for phase in ("opening", "middlegame", "endgame")
    for material in ("stm_behind", "balanced", "stm_ahead")
)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def bind(path: Path) -> dict[str, str]:
    if not path.is_file():
        raise ValueError(f"missing input: {path}")
    return {"path": str(path), "sha256": sha256(path)}


def read(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def read_jsonl(path: Path) -> list[dict[str, Any]]:
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line]


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def stratum(row: dict[str, Any]) -> str:
    tags = row.get("tags", {})
    value = f"{tags.get('phase')}/{tags.get('material_band')}"
    require(value in STRATA, f"unsupported phase/material stratum: {value}")
    return value


def stable_rank(row: dict[str, Any], seed: int) -> str:
    source = row.get("source", {})
    value = f"{seed}\0{source.get('source_key')}\0{row.get('sfen')}"
    return hashlib.sha256(value.encode()).hexdigest()


def select(args: argparse.Namespace) -> dict[str, Any]:
    q21h = read(args.q21h_manifest)
    excluded_corpus = read(args.excluded_corpus)
    prior_decision = read(args.prior_decision)
    require(
        q21h.get("schema") == "sekirei.q21h-independent-learning-split.v1"
        and q21h.get("selection_ready") is True,
        "Q21h split is not frozen and ready",
    )
    require(
        prior_decision.get("schema") == "sekirei.q21p-depth7-ranking-pilot-decision.v2"
        and prior_decision.get("status") == "fail",
        "Q21p retry requires the preserved v2 FAIL",
    )
    source_path = Path(q21h[args.source_arm]["path"])
    require(
        sha256(source_path) == q21h[args.source_arm]["sha256"],
        f"Q21h {args.source_arm} SHA mismatch",
    )
    excluded_positions = excluded_corpus.get("positions", [])
    excluded_sfen = {row["sfen"] for row in excluded_positions}
    excluded_groups = {
        row.get("source", {}).get("derived_group") for row in excluded_positions
    } - {None}

    selected: list[dict[str, Any]] = []
    selected_groups: set[str] = set()
    rows = read_jsonl(source_path)
    for wanted in STRATA:
        bucket = sorted(
            (
                row
                for row in rows
                if stratum(row) == wanted
                and row.get("sfen") not in excluded_sfen
                and row.get("source", {}).get("derived_group") not in excluded_groups
            ),
            key=lambda row: stable_rank(row, args.seed),
        )
        for row in bucket:
            group = row.get("source", {}).get("derived_group")
            if not isinstance(group, str) or group in selected_groups:
                continue
            selected.append(row)
            selected_groups.add(group)
            if sum(stratum(item) == wanted for item in selected) == args.per_stratum:
                break
        require(
            sum(stratum(item) == wanted for item in selected) == args.per_stratum,
            f"{wanted}: insufficient fresh derived groups",
        )

    positions = [
        {
            "id": f"q21p-v3-val-{index:02d}",
            "category": stratum(row),
            "initial_sfen": row["sfen"],
            "history_before_usi": [],
            "sfen": row["sfen"],
            "source": row["source"],
        }
        for index, row in enumerate(selected, 1)
    ]
    corpus = {
        "schema": "sekirei.q21p-retry-validation-corpus.v1",
        "diagnostic_only": True,
        "strength_claim": False,
        "selection": {
            "score_blind": True,
            "source_arm": f"Q21h {args.source_arm} only",
            "seed": args.seed,
            "per_phase_material_stratum": args.per_stratum,
            "positions": len(positions),
            "counts": dict(sorted(Counter(row["category"] for row in positions).items())),
            "excluded_training_sfens": len(excluded_sfen),
            "excluded_training_derived_groups": len(excluded_groups),
            "unique_derived_groups": len(selected_groups),
        },
        "positions": positions,
    }
    args.output_dir.mkdir(parents=True, exist_ok=True)
    corpus_path = args.output_dir / "validation-corpus.json"
    corpus_path.write_text(json.dumps(corpus, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    document = {
        "schema": "sekirei.q21p-retry-holdout-selection.v1",
        "status": "frozen_before_any_teacher_label",
        "diagnostic_only": True,
        "strength_claim": False,
        "inputs": {
            "q21h_manifest": bind(args.q21h_manifest),
            "q21h_source": bind(source_path),
            "excluded_corpus": bind(args.excluded_corpus),
            "prior_decision": bind(args.prior_decision),
        },
        "contract": corpus["selection"],
        "validation_corpus": bind(corpus_path),
    }
    selection_path = args.output_dir / "holdout-selection.json"
    selection_path.write_text(
        json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return document


def filter_eligible(args: argparse.Namespace) -> dict[str, Any]:
    reserve_selection = read(args.reserve_selection)
    reserve_corpus = read(Path(reserve_selection["validation_corpus"]["path"]))
    shallow = read(args.shallow_teacher)
    require(
        reserve_selection.get("schema") == "sekirei.q21p-retry-holdout-selection.v1"
        and reserve_selection.get("status") == "frozen_before_any_teacher_label",
        "reserve hold-out selection is invalid",
    )
    require(
        shallow.get("schema") == "sekirei.root-rank-teacher-corpus.v1"
        and shallow.get("contract", {}).get("depth") == 3
        and shallow.get("contract", {}).get("complete_legal_root_set") is True
        and shallow.get("source_corpus", {}).get("sha256")
        == reserve_selection["validation_corpus"]["sha256"],
        "reserve shallow labels are incomplete or unbound",
    )
    shallow_by_id = {row["id"]: row for row in shallow["rows"]}
    selected_positions: list[dict[str, Any]] = []
    selected_ids: set[str] = set()
    excluded = []
    for wanted in STRATA:
        for position in (row for row in reserve_corpus["positions"] if row["category"] == wanted):
            teacher_row = shallow_by_id[position["id"]]
            legal_moves = teacher_row.get("teacher_root", {}).get("root_legal_move_count")
            if not isinstance(legal_moves, int) or legal_moves < 2:
                excluded.append({"id": position["id"], "category": wanted, "legal_moves": legal_moves})
                continue
            selected_positions.append(position)
            selected_ids.add(position["id"])
            if sum(row["category"] == wanted for row in selected_positions) == args.per_stratum:
                break
        require(
            sum(row["category"] == wanted for row in selected_positions) == args.per_stratum,
            f"{wanted}: reserve has fewer than {args.per_stratum} positions with at least two legal moves",
        )
    corpus = {
        **reserve_corpus,
        "schema": "sekirei.q21p-retry-validation-corpus.v2",
        "selection": {
            **reserve_corpus["selection"],
            "reserve_positions": len(reserve_corpus["positions"]),
            "positions": len(selected_positions),
            "per_phase_material_stratum": args.per_stratum,
            "eligibility_filter": "completed depth-3 root with at least two legal moves; scores ignored",
            "excluded": excluded,
        },
        "positions": selected_positions,
    }
    args.output_dir.mkdir(parents=True, exist_ok=True)
    corpus_path = args.output_dir / "validation-corpus.json"
    corpus_path.write_text(json.dumps(corpus, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    filtered_shallow = {
        **shallow,
        "source_corpus": bind(corpus_path),
        "rows": [row for row in shallow["rows"] if row["id"] in selected_ids],
    }
    shallow_path = args.output_dir / "validation-shallow-teacher.json"
    shallow_path.write_text(
        json.dumps(filtered_shallow, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    document = {
        "schema": "sekirei.q21p-retry-holdout-selection.v2",
        "status": "filtered_by_preregistered_legality_only",
        "diagnostic_only": True,
        "strength_claim": False,
        "selection_used_scores": False,
        "inputs": {
            "reserve_selection": bind(args.reserve_selection),
            "reserve_corpus": bind(Path(reserve_selection["validation_corpus"]["path"])),
            "reserve_shallow_teacher": bind(args.shallow_teacher),
        },
        "contract": corpus["selection"],
        "validation_corpus": bind(corpus_path),
        "shallow_teacher": bind(shallow_path),
    }
    selection_path = args.output_dir / "holdout-selection.json"
    selection_path.write_text(
        json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return document


def freeze(args: argparse.Namespace) -> dict[str, Any]:
    selection = read(args.selection)
    shallow = read(args.shallow_teacher)
    q21q = read(args.q21q_diagnostic)
    prior_decision = read(args.prior_decision)
    train_pairs = read(args.train_pairs)
    selection_v2 = (
        selection.get("schema") == "sekirei.q21p-retry-holdout-selection.v2"
        and selection.get("status") == "filtered_by_preregistered_legality_only"
        and selection.get("selection_used_scores") is False
    )
    require(selection_v2, "fresh hold-out was not frozen and legality-filtered before depth-7 labels")
    corpus_path = Path(selection["validation_corpus"]["path"])
    require(selection["validation_corpus"]["sha256"] == sha256(corpus_path), "hold-out corpus SHA mismatch")
    require(
        shallow.get("schema") == "sekirei.root-rank-teacher-corpus.v1"
        and shallow.get("contract", {}).get("depth") == 3
        and shallow.get("contract", {}).get("complete_legal_root_set") is True
        and shallow.get("source_corpus", {}).get("sha256") == sha256(corpus_path)
        and len(shallow.get("rows", [])) == selection["contract"]["positions"],
        "shallow candidate-source labels are incomplete or unbound",
    )
    require(
        q21q.get("schema") == "sekirei.q21q-transfer-boundary-diagnostic.v1"
        and q21q.get("status") == "pass"
        and q21q.get("selected_recipe", {}).get("selection_used_validation") is False,
        "Q21q did not select a train-only recipe",
    )
    require(
        prior_decision.get("schema") == "sekirei.q21p-depth7-ranking-pilot-decision.v2"
        and prior_decision.get("status") == "fail",
        "Q21p retry requires the preserved v2 FAIL",
    )
    require(train_pairs.get("schema") == "sekirei.root-rank-pairs.v1", "invalid train pair corpus")
    recipe = q21q["selected_recipe"]
    selected = read(args.q21o_decision)["selected_contract"]
    require(
        selected.get("arm") == "depth7"
        and selected.get("max_depth") == 7
        and selected.get("threads") == 1
        and selected.get("spec_top_n") == 0,
        "Q21o depth-7 contract mismatch",
    )
    require(selected["weights_sha256"] == sha256(args.weights), "teacher weight SHA mismatch")
    require(selected["binary_sha256"] == sha256(args.engine), "engine SHA mismatch")
    document = {
        "schema": "sekirei.q21p-depth7-ranking-pilot-preregistration.v4",
        "status": "frozen_before_depth7_validation_labels_and_candidate_training",
        "diagnostic_only": True,
        "strength_claim": False,
        "hypothesis": (
            "The corrected residual-material ranking objective transfers after updates cross the "
            "quantized-inference ranking boundary identified using training pairs only."
        ),
        "single_factor": "ranking optimization budget: 3 epochs at 1e-4 -> 30 epochs at 1e-3",
        "parents": selection["contract"]["positions"],
        "candidate_contract": {
            "mode": "preregistered_candidate_union",
            "members": "depth-3 complete-root top 8 union every depth-7 free bestmove repeat",
            "maximum_moves_per_parent": 10,
            "free_repeats": 2,
            "fixed_root_repeats": 2,
            "top_k_after_depth7_reranking": 8,
            "pair_selection": "adjacent",
            "normal_score_abs_max_cp": 10_000,
            "require_exact_completed_search": True,
            "require_aa_signature_match": True,
        },
        "teacher_contract": {**selected, "cold_process_per_search": True, "timeout_seconds_per_search": 600},
        "training_contract": {
            "initial_weights_sha256": selected["weights_sha256"],
            "fresh_optimizer": True,
            "init_seed": 42,
            "learning_rate": recipe["learning_rate"],
            "epochs": recipe["epochs"],
            "ranking_parent_balanced": True,
            "ranking_batch_pairs": 1,
            "nnue_output": "residual-material",
            "no_scalar_targets": True,
            "selection_used_validation": False,
        },
        "validation_contract": {
            "fresh_holdout": True,
            "baseline": "initial fixed-T checkpoint on the generated depth-7 pairs",
            "minimum_mean_parent_rank_loss_reduction": 0.10,
            "major_blunders_ge_300cp_must_not_increase": True,
            "development_match_only_after_screen_pass": True,
        },
        "authorization": {
            "screen_pass": "Q21p complete; authorize only the preregistered 32-game development match",
            "screen_fail": "reject candidate; no development match and no Q20",
            "q20_authorized": False,
        },
        "inputs": {
            "selection": bind(args.selection),
            "corpus": bind(corpus_path),
            "shallow_teacher": bind(args.shallow_teacher),
            "q21q_diagnostic": bind(args.q21q_diagnostic),
            "q21o_decision": bind(args.q21o_decision),
            "prior_decision": bind(args.prior_decision),
            "superseded_infeasible_preregistration": bind(args.superseded_preregistration),
            "train_pairs": bind(args.train_pairs),
            "weights": bind(args.weights),
            "engine": bind(args.engine),
            "trainer": bind(args.trainer),
            "ranking_auditor": bind(args.auditor),
        },
        "tools": {
            "preparer": bind(Path(__file__).resolve()),
            "runner": bind(args.runner),
            "pair_builder": bind(args.pair_builder),
            "finalizer": bind(args.finalizer),
        },
    }
    args.output.write_text(
        json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return document


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    select_parser = subparsers.add_parser("select")
    select_parser.add_argument("--q21h-manifest", type=Path, required=True)
    select_parser.add_argument("--excluded-corpus", type=Path, required=True)
    select_parser.add_argument("--source-arm", choices=("train", "validation"), default="train")
    select_parser.add_argument("--prior-decision", type=Path, required=True)
    select_parser.add_argument("--output-dir", type=Path, required=True)
    select_parser.add_argument("--seed", type=int, default=212103)
    select_parser.add_argument("--per-stratum", type=int, default=2)

    filter_parser = subparsers.add_parser("filter")
    filter_parser.add_argument("--reserve-selection", type=Path, required=True)
    filter_parser.add_argument("--shallow-teacher", type=Path, required=True)
    filter_parser.add_argument("--output-dir", type=Path, required=True)
    filter_parser.add_argument("--per-stratum", type=int, default=2)

    freeze_parser = subparsers.add_parser("freeze")
    for name in (
        "selection", "shallow-teacher", "q21q-diagnostic", "q21o-decision",
        "prior-decision", "superseded-preregistration", "train-pairs", "weights", "engine", "trainer", "auditor",
        "runner", "pair-builder", "finalizer", "output",
    ):
        freeze_parser.add_argument(f"--{name}", type=Path, required=True)
    args = parser.parse_args()
    try:
        if args.command == "select":
            document = select(args)
        elif args.command == "filter":
            document = filter_eligible(args)
        else:
            document = freeze(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    print(json.dumps({"schema": document["schema"], "status": document["status"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
