#!/usr/bin/env python3
"""Freeze Q21u's fresh validation holdout after its train recipe is fixed."""

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


def jsonl(path: Path) -> list[dict[str, Any]]:
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
    key = f"{seed}\0{source.get('source_key')}\0{row.get('sfen')}"
    return hashlib.sha256(key.encode()).hexdigest()


def corpus_positions(document: dict[str, Any]) -> list[dict[str, Any]]:
    positions = document.get("positions", [])
    require(isinstance(positions, list), "excluded corpus lacks positions")
    return positions


def reserve(args: argparse.Namespace) -> dict[str, Any]:
    recipe = read(args.recipe_decision)
    require(
        recipe.get("schema") == "sekirei.q21u-listwise-train-recipe-decision.v1"
        and recipe.get("status") == "recipe_frozen"
        and recipe.get("minimum_progress_pass") is True
        and recipe.get("validation_inspected") is False,
        "Q21u train-only recipe is not frozen before validation",
    )
    split = read(args.q21h_manifest)
    require(
        split.get("schema") == "sekirei.q21h-independent-learning-split.v1"
        and split.get("selection_ready") is True,
        "Q21h split is not frozen",
    )
    source = Path(split["train"]["path"])
    require(sha256(source) == split["train"]["sha256"], "Q21h train SHA mismatch")

    excluded_sfens: set[str] = set()
    excluded_groups: set[str] = set()
    excluded_bindings = []
    for path in args.exclude_corpus:
        document = read(path)
        excluded_bindings.append(bind(path))
        for position in corpus_positions(document):
            sfen = position.get("sfen") or position.get("parent_sfen")
            group = position.get("source", {}).get("derived_group")
            if isinstance(sfen, str):
                excluded_sfens.add(sfen)
            if isinstance(group, str):
                excluded_groups.add(group)

    source_rows = jsonl(source)
    buckets = {
        wanted: sorted(
            (
                row
                for row in source_rows
                if stratum(row) == wanted
                and row.get("sfen") not in excluded_sfens
                and row.get("source", {}).get("derived_group") not in excluded_groups
            ),
            key=lambda row: stable_rank(row, args.seed),
        )
        for wanted in STRATA
    }
    selected = []
    group_owner: dict[str, str] = {}
    # One derived group can cover several strata. Allocate the rarest first,
    # but allow several reserve positions from one group within that stratum.
    # Only one position is retained after the legality-only filter.
    for wanted in sorted(
        STRATA,
        key=lambda category: (
            len({row.get("source", {}).get("derived_group") for row in buckets[category]}),
            category,
        ),
    ):
        selected_sfens: set[str] = set()
        for row in buckets[wanted]:
            group = row.get("source", {}).get("derived_group")
            if (
                not isinstance(group, str)
                or (group in group_owner and group_owner[group] != wanted)
                or row["sfen"] in selected_sfens
            ):
                continue
            selected.append(row)
            selected_sfens.add(row["sfen"])
            group_owner[group] = wanted
            if len(selected_sfens) == args.reserve_per_stratum:
                break
        require(selected_sfens, f"{wanted}: no fresh derived group")

    positions = [
        {
            "id": f"q21u-reserve-{index:02d}",
            "category": stratum(row),
            "initial_sfen": row["sfen"],
            "history_before_usi": [],
            "sfen": row["sfen"],
            "source": row["source"],
        }
        for index, row in enumerate(sorted(selected, key=lambda row: STRATA.index(stratum(row))), 1)
    ]
    corpus = {
        "schema": "sekirei.q21u-validation-reserve.v1",
        "diagnostic_only": True,
        "strength_claim": False,
        "selection": {
            "score_blind": True,
            "after_recipe_freeze": True,
            "seed": args.seed,
            "reserve_per_stratum_maximum": args.reserve_per_stratum,
            "excluded_sfens": len(excluded_sfens),
            "excluded_derived_groups": len(excluded_groups),
            "positions": len(positions),
            "counts": dict(sorted(Counter(position["category"] for position in positions).items())),
        },
        "positions": positions,
    }
    args.output_dir.mkdir(parents=True, exist_ok=True)
    corpus_path = args.output_dir / "validation-reserve.json"
    corpus_path.write_text(json.dumps(corpus, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    document = {
        "schema": "sekirei.q21u-holdout-selection.v1",
        "status": "frozen_before_any_validation_teacher_label",
        "diagnostic_only": True,
        "strength_claim": False,
        "selection_used_scores": False,
        "recipe_decision": bind(args.recipe_decision),
        "selected_candidate": recipe["selected"]["candidate"],
        "inputs": {
            "q21h_manifest": bind(args.q21h_manifest),
            "q21h_train": bind(source),
            "excluded_corpora": excluded_bindings,
        },
        "contract": corpus["selection"],
        "reserve_corpus": bind(corpus_path),
    }
    (args.output_dir / "holdout-selection.json").write_text(
        json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return document


def freeze(args: argparse.Namespace) -> dict[str, Any]:
    selection = read(args.selection)
    shallow = read(args.shallow_teacher)
    teacher_decision = read(args.teacher_decision)
    recipe = read(args.recipe_decision)
    require(
        selection.get("schema") == "sekirei.q21u-holdout-selection.v1"
        and selection.get("status") == "frozen_before_any_validation_teacher_label"
        and selection.get("selection_used_scores") is False,
        "Q21u validation reserve was not frozen score-blind",
    )
    require(
        selection["recipe_decision"]["sha256"] == sha256(args.recipe_decision)
        and recipe.get("status") == "recipe_frozen",
        "recipe decision changed after holdout selection",
    )
    reserve_path = Path(selection["reserve_corpus"]["path"])
    require(selection["reserve_corpus"]["sha256"] == sha256(reserve_path), "reserve SHA mismatch")
    reserve = read(reserve_path)
    require(
        shallow.get("schema") == "sekirei.root-rank-teacher-corpus.v1"
        and shallow.get("contract", {}).get("depth") == 3
        and shallow.get("source_corpus", {}).get("sha256") == sha256(reserve_path),
        "shallow teacher is not bound to the Q21u reserve",
    )
    shallow_by_id = {row["id"]: row for row in shallow.get("rows", [])}
    require(set(shallow_by_id) == {row["id"] for row in reserve["positions"]}, "shallow parent set mismatch")
    chosen = []
    excluded = []
    for wanted in STRATA:
        for position in (row for row in reserve["positions"] if row["category"] == wanted):
            label = shallow_by_id[position["id"]]
            count = label.get("teacher_root", {}).get("root_legal_move_count")
            if label.get("complete_legal_root_set") is not True or not isinstance(count, int) or count < 2:
                excluded.append(
                    {
                        "id": position["id"],
                        "category": wanted,
                        "legal_moves": count,
                        "complete": label.get("complete_legal_root_set") is True,
                    }
                )
                continue
            chosen.append(position)
            break
        require(any(row["category"] == wanted for row in chosen), f"{wanted}: no legality-eligible fresh reserve")
    chosen_groups = [row.get("source", {}).get("derived_group") for row in chosen]
    require(len(set(chosen_groups)) == len(chosen_groups), "chosen strata share a derived group")
    for position in chosen:
        label = shallow_by_id[position["id"]]
        count = label.get("teacher_root", {}).get("root_legal_move_count")
        require(
            label.get("complete_legal_root_set") is True and isinstance(count, int) and count >= 2,
            f"{position['id']}: fresh holdout has fewer than two complete legal moves",
        )

    selected = teacher_decision.get("selected_contract", {})
    require(
        selected.get("arm") == "depth7"
        and selected.get("max_depth") == 7
        and selected.get("threads") == 1
        and selected.get("spec_top_n") == 0,
        "Q21o depth-7 teacher contract mismatch",
    )
    require(selected["weights_sha256"] == sha256(args.weights), "teacher weight mismatch")
    require(selected["binary_sha256"] == sha256(args.engine), "teacher engine mismatch")
    require(recipe["selected"]["candidate"]["sha256"] == sha256(args.candidate), "candidate mismatch")

    corpus = {
        "schema": "sekirei.q21u-validation-corpus.v1",
        "diagnostic_only": True,
        "strength_claim": False,
        "selection": {
            "score_blind": True,
            "legality_only_filter": True,
            "per_stratum": 1,
            "positions": len(chosen),
            "excluded": excluded,
        },
        "positions": chosen,
    }
    corpus_path = args.output_dir / "validation-corpus.json"
    corpus_path.write_text(json.dumps(corpus, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    chosen_ids = {row["id"] for row in chosen}
    shallow_bound = {
        **shallow,
        "source_corpus": bind(corpus_path),
        "rows": [row for row in shallow["rows"] if row["id"] in chosen_ids],
    }
    shallow_path = args.output_dir / "validation-shallow-teacher.json"
    shallow_path.write_text(json.dumps(shallow_bound, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

    document = {
        "schema": "sekirei.q21u-listwise-validation-preregistration.v1",
        "status": "frozen_before_depth7_labels_and_validation_screen",
        "diagnostic_only": True,
        "strength_claim": False,
        "single_factor": "Q21u frozen listwise recipe versus its unchanged initial checkpoint",
        "parents": len(chosen),
        "candidate_contract": {
            "mode": "preregistered_candidate_union",
            "members": "depth-3 complete-root top 8 union every depth-7 free bestmove repeat",
            "maximum_moves_per_parent": 10,
            "free_repeats": 2,
            "fixed_root_repeats": 2,
            "top_k_after_depth7_reranking": 8,
            "pair_selection": "top-vs-rest",
            "normal_score_abs_max_cp": 10_000,
            "require_exact_completed_search": True,
            "require_aa_signature_match": True,
        },
        "teacher_contract": {**selected, "cold_process_per_search": True, "timeout_seconds_per_search": 600},
        "screen_contract": {
            "static_mean_direct_top_regret_reduction_minimum": 0.10,
            "static_top1_matches_must_not_decrease": True,
            "static_major_regret_ge_300_must_not_increase": True,
            "same_time_candidate_mean_regret_must_not_exceed_material": True,
            "same_time_candidate_major_regret_must_not_exceed_material": True,
            "same_time_candidate_top1_matches_must_not_be_below_material": True,
            "development_match_only_if_all_conditions_pass": True,
        },
        "inputs": {
            "selection": bind(args.selection),
            "recipe_decision": bind(args.recipe_decision),
            "corpus": bind(corpus_path),
            "shallow_teacher": bind(shallow_path),
            "engine": bind(args.engine),
            "weights": bind(args.weights),
            "candidate": bind(args.candidate),
            "teacher_decision": bind(args.teacher_decision),
            "ranking_auditor": bind(args.ranking_auditor),
        },
        "tools": {
            "label_runner": bind(args.label_runner),
            "pair_builder": bind(args.pair_builder),
            "screen_runner": bind(args.screen_runner),
            "preparer": bind(Path(__file__).resolve()),
        },
    }
    prereg_path = args.output_dir / "validation-preregistration.json"
    prereg_path.write_text(
        json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return document


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    reserve_parser = subparsers.add_parser("reserve")
    reserve_parser.add_argument("--recipe-decision", type=Path, required=True)
    reserve_parser.add_argument("--q21h-manifest", type=Path, required=True)
    reserve_parser.add_argument("--exclude-corpus", type=Path, action="append", required=True)
    reserve_parser.add_argument("--output-dir", type=Path, required=True)
    reserve_parser.add_argument("--seed", type=int, default=212105)
    reserve_parser.add_argument("--reserve-per-stratum", type=int, default=3)
    freeze_parser = subparsers.add_parser("freeze")
    freeze_parser.add_argument("--selection", type=Path, required=True)
    freeze_parser.add_argument("--shallow-teacher", type=Path, required=True)
    freeze_parser.add_argument("--teacher-decision", type=Path, required=True)
    freeze_parser.add_argument("--recipe-decision", type=Path, required=True)
    freeze_parser.add_argument("--weights", type=Path, required=True)
    freeze_parser.add_argument("--candidate", type=Path, required=True)
    freeze_parser.add_argument("--engine", type=Path, required=True)
    freeze_parser.add_argument("--ranking-auditor", type=Path, required=True)
    freeze_parser.add_argument("--label-runner", type=Path, required=True)
    freeze_parser.add_argument("--pair-builder", type=Path, required=True)
    freeze_parser.add_argument("--screen-runner", type=Path, required=True)
    freeze_parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    try:
        document = reserve(args) if args.command == "reserve" else freeze(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    print(json.dumps({"schema": document["schema"], "status": document["status"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
