#!/usr/bin/env python3
"""Freeze Q21t's fresh holdout and top-choice pilot contract."""

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


def rows(path: Path) -> list[dict[str, Any]]:
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
            if isinstance(sfen, str):
                excluded_sfens.add(sfen)
            group = position.get("source", {}).get("derived_group")
            if isinstance(group, str):
                excluded_groups.add(group)
    selected = []
    selected_groups: set[str] = set()
    source_rows = rows(source)
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
    # A derived group may contain several phase/material strata.  Allocate the
    # scarcest stratum first so a plentiful earlier bucket cannot consume the
    # only independent group available to a rare stratum.
    selection_order = sorted(
        STRATA,
        key=lambda wanted: (
            len({row.get("source", {}).get("derived_group") for row in buckets[wanted]}),
            wanted,
        ),
    )
    for wanted in selection_order:
        bucket = buckets[wanted]
        for row in bucket:
            group = row.get("source", {}).get("derived_group")
            if not isinstance(group, str) or group in selected_groups:
                continue
            selected.append(row)
            selected_groups.add(group)
            if sum(stratum(item) == wanted for item in selected) == args.reserve_per_stratum:
                break
        require(
            sum(stratum(item) == wanted for item in selected) == args.reserve_per_stratum,
            f"{wanted}: insufficient unused derived groups",
        )
    positions = [
        {
            "id": f"q21t-val-{index:02d}",
            "category": stratum(row),
            "initial_sfen": row["sfen"],
            "history_before_usi": [],
            "sfen": row["sfen"],
            "source": row["source"],
        }
        for index, row in enumerate(selected, 1)
    ]
    corpus = {
        "schema": "sekirei.q21t-validation-reserve.v1",
        "diagnostic_only": True,
        "strength_claim": False,
        "selection": {
            "score_blind": True,
            "seed": args.seed,
            "reserve_per_stratum": args.reserve_per_stratum,
            "excluded_sfens": len(excluded_sfens),
            "excluded_derived_groups": len(excluded_groups),
            "positions": len(positions),
            "counts": dict(sorted(Counter(item["category"] for item in positions).items())),
        },
        "positions": positions,
    }
    args.output_dir.mkdir(parents=True, exist_ok=True)
    corpus_path = args.output_dir / "validation-reserve.json"
    corpus_path.write_text(json.dumps(corpus, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    document = {
        "schema": "sekirei.q21t-holdout-selection.v1",
        "status": "frozen_before_any_teacher_label",
        "diagnostic_only": True,
        "strength_claim": False,
        "selection_used_scores": False,
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
    q21o = read(args.q21o_decision)
    require(
        selection.get("schema") == "sekirei.q21t-holdout-selection.v1"
        and selection.get("status") == "frozen_before_any_teacher_label"
        and selection.get("selection_used_scores") is False,
        "Q21t reserve was not frozen score-blind",
    )
    reserve_path = Path(selection["reserve_corpus"]["path"])
    reserve = read(reserve_path)
    require(selection["reserve_corpus"]["sha256"] == sha256(reserve_path), "reserve SHA mismatch")
    require(
        shallow.get("schema") == "sekirei.root-rank-teacher-corpus.v1"
        and shallow.get("contract", {}).get("depth") == 3
        and shallow.get("source_corpus", {}).get("sha256") == sha256(reserve_path),
        "shallow teacher is not bound to the reserve",
    )
    shallow_by_id = {row["id"]: row for row in shallow["rows"]}
    chosen = []
    excluded = []
    for wanted in STRATA:
        for position in (row for row in reserve["positions"] if row["category"] == wanted):
            label = shallow_by_id[position["id"]]
            count = label.get("teacher_root", {}).get("root_legal_move_count")
            complete = label.get("complete_legal_root_set") is True
            if not complete or not isinstance(count, int) or count < 2:
                excluded.append(
                    {"id": position["id"], "category": wanted, "legal_moves": count, "complete": complete}
                )
                continue
            chosen.append(position)
            break
        require(any(row["category"] == wanted for row in chosen), f"{wanted}: no eligible reserve")
    corpus = {
        "schema": "sekirei.q21t-validation-corpus.v1",
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
    args.output_dir.mkdir(parents=True, exist_ok=True)
    corpus_path = args.output_dir / "validation-corpus.json"
    corpus_path.write_text(json.dumps(corpus, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    chosen_ids = {row["id"] for row in chosen}
    shallow_filtered = {
        **shallow,
        "source_corpus": bind(corpus_path),
        "rows": [row for row in shallow["rows"] if row["id"] in chosen_ids],
    }
    shallow_path = args.output_dir / "validation-shallow-teacher.json"
    shallow_path.write_text(
        json.dumps(shallow_filtered, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    selected = q21o.get("selected_contract", {})
    require(
        selected.get("arm") == "depth7"
        and selected.get("max_depth") == 7
        and selected.get("threads") == 1
        and selected.get("spec_top_n") == 0,
        "Q21o depth-7 contract mismatch",
    )
    require(selected["weights_sha256"] == sha256(args.weights), "teacher weight mismatch")
    require(selected["binary_sha256"] == sha256(args.engine), "engine mismatch")
    document = {
        "schema": "sekirei.q21t-top-choice-pilot-preregistration.v1",
        "status": "frozen_before_depth7_labels_and_candidate_training",
        "diagnostic_only": True,
        "strength_claim": False,
        "hypothesis": (
            "Direct teacher-top-versus-rest training and a top-choice screen align the pilot with "
            "decision quality better than adjacent-pair mean rank loss."
        ),
        "single_factor": "pair objective and screen: adjacent local gaps -> direct top-versus-rest gaps",
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
        "training_contract": {
            "initial_weights_sha256": selected["weights_sha256"],
            "fresh_optimizer": True,
            "seed": 42,
            "learning_rate": 0.001,
            "epochs": 30,
            "parent_balanced": True,
            "nnue_output": "residual-material",
        },
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
            "corpus": bind(corpus_path),
            "shallow_teacher": bind(shallow_path),
            "engine": bind(args.engine),
            "weights": bind(args.weights),
            "q21o_decision": bind(args.q21o_decision),
            "train_pairs": bind(args.train_pairs),
            "trainer": bind(args.trainer),
            "ranking_auditor": bind(args.ranking_auditor),
        },
        "tools": {
            "runner": bind(args.label_runner),
            "pair_builder": bind(args.pair_builder),
            "preparer": bind(Path(__file__).resolve()),
        },
    }
    (args.output_dir / "preregistration.json").write_text(
        json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return document


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    select_parser = subparsers.add_parser("reserve")
    select_parser.add_argument("--q21h-manifest", type=Path, required=True)
    select_parser.add_argument("--exclude-corpus", type=Path, action="append", required=True)
    select_parser.add_argument("--output-dir", type=Path, required=True)
    select_parser.add_argument("--seed", type=int, default=212104)
    select_parser.add_argument("--reserve-per-stratum", type=int, default=3)
    freeze_parser = subparsers.add_parser("freeze")
    freeze_parser.add_argument("--selection", type=Path, required=True)
    freeze_parser.add_argument("--shallow-teacher", type=Path, required=True)
    freeze_parser.add_argument("--q21o-decision", type=Path, required=True)
    freeze_parser.add_argument("--weights", type=Path, required=True)
    freeze_parser.add_argument("--engine", type=Path, required=True)
    freeze_parser.add_argument("--train-pairs", type=Path, required=True)
    freeze_parser.add_argument("--trainer", type=Path, required=True)
    freeze_parser.add_argument("--ranking-auditor", type=Path, required=True)
    freeze_parser.add_argument("--label-runner", type=Path, required=True)
    freeze_parser.add_argument("--pair-builder", type=Path, required=True)
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
