#!/usr/bin/env python3
"""Select Q28's fresh score-blind tactical-balanced parent boundary."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any

import prepare_q21w_coverage as q21w
from preregister_q28_tactical_source_family import SCHEMA, bind, sha256


POOL_SCHEMA = "sekirei.q28-tactical-pool-scan.v1"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def identity(row: dict[str, Any]) -> str:
    return q21w.q21h.symmetry_key(str(row["sfen"]))


def rank(seed: int, category: str, purpose: str, row: dict[str, Any]) -> int:
    source = row["source"]
    return q21w.stable_rank(seed, f"q28-{category}-{purpose}", f"{source['source_key']}\0{row['sfen']}")


def select_group(
    rows: list[dict[str, Any]],
    *,
    category: str,
    seed: int,
    minimum_nonchecking: int,
    used_sources: set[str],
    used_identities: set[str],
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    nonchecking = {"capture_resource", "evasion", "quiet"}
    selected_train: list[dict[str, Any]] = []
    selected_holdout: list[dict[str, Any]] = []

    def take(candidates: list[dict[str, Any]], destination: list[dict[str, Any]], limit: int, purpose: str) -> None:
        for row in sorted(candidates, key=lambda item: rank(seed, category, purpose, item)):
            source_key = str(row["source"]["source_key"])
            row_identity = identity(row)
            if source_key in used_sources or row_identity in used_identities:
                continue
            destination.append(row)
            used_sources.add(source_key)
            used_identities.add(row_identity)
            if len(destination) == limit:
                return
        raise ValueError(f"{category}: insufficient score-blind candidates for {purpose}")

    take(
        [row for row in rows if row["tactical_class"] in nonchecking],
        selected_train,
        minimum_nonchecking,
        "nonchecking",
    )
    take(rows, selected_train, 8, "train")
    take(rows, selected_holdout, 2, "holdout")
    return selected_train, selected_holdout


def corpus(kind: str, rows: list[dict[str, Any]]) -> dict[str, Any]:
    return {
        "schema": f"sekirei.q28-{kind}-reserve.v1",
        "diagnostic_only": True,
        "strength_claim": False,
        "selection_used_scores": False,
        "positions": [
            {
                "id": f"q28-{kind}-{index:03d}",
                "category": row["category"],
                "initial_sfen": row["sfen"],
                "history_before_usi": [],
                "sfen": row["sfen"],
                "source": row["source"],
                "tactical_class": row["tactical_class"],
                "rule_facts": row["rule_facts"],
            }
            for index, row in enumerate(sorted(rows, key=lambda item: (item["category"], item["source"]["source_key"])), 1)
        ],
    }


def prepare(args: argparse.Namespace) -> dict[str, Any]:
    family = json.loads(args.family_preregistration.read_text(encoding="utf-8"))
    pool = json.loads(args.pool_scan.read_text(encoding="utf-8"))
    require(
        family.get("schema") == SCHEMA and family.get("status") == "frozen_before_score_blind_parent_selection",
        "family preregistration is invalid",
    )
    require(pool.get("schema") == POOL_SCHEMA and pool.get("status") == "complete", "pool scan is invalid")
    expected_pool = family["inputs"]["unused_score_free_pool"]
    require(expected_pool == bind(args.pool_scan), "pool scan differs from frozen family")
    contract = family["source_selection"]
    seed = int(contract["seed"])
    minimum = int(contract["minimum_nonchecking_train_per_stratum"])
    grouped: dict[str, list[dict[str, Any]]] = {stratum: [] for stratum in q21w.STRATA}
    for row in pool["rows"]:
        category = row.get("category")
        require(category in grouped, f"unexpected category {category!r}")
        grouped[category].append(row)

    used_sources: set[str] = set()
    used_identities: set[str] = set()
    train_rows: list[dict[str, Any]] = []
    holdout_rows: list[dict[str, Any]] = []
    for category in q21w.STRATA:
        train, holdout = select_group(
            grouped[category],
            category=category,
            seed=seed,
            minimum_nonchecking=minimum,
            used_sources=used_sources,
            used_identities=used_identities,
        )
        train_rows.extend(train)
        holdout_rows.extend(holdout)
    require(len(train_rows) == 72 and len(holdout_rows) == 18, "parent count drift")
    train = corpus("train", train_rows)
    holdout = corpus("holdout", holdout_rows)
    train_classes = {
        category: sum(row["tactical_class"] != "checking_resource" for row in train_rows if row["category"] == category)
        for category in q21w.STRATA
    }
    require(all(count >= minimum for count in train_classes.values()), "nonchecking quota drift")
    return {
        "schema": "sekirei.q28-execution-preregistration.v1",
        "status": "frozen_before_any_self_teacher_label",
        "diagnostic_only": True,
        "strength_claim": False,
        "inputs": {"family": bind(args.family_preregistration), "pool_scan": bind(args.pool_scan)},
        "selection": {
            "used_scores": False,
            "selected_sources": len(used_sources),
            "selected_symmetric_positions": len(used_identities),
            "train_nonchecking_per_stratum": train_classes,
        },
        "artifacts": {"train": train, "holdout": holdout},
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--family-preregistration", type=Path, required=True)
    parser.add_argument("--pool-scan", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    if args.output_dir.exists():
        parser.error("output directory already exists; execution boundary is immutable")
    result = prepare(args)
    args.output_dir.mkdir(parents=True)
    (args.output_dir / "preregistration.json").write_text(
        json.dumps({key: value for key, value in result.items() if key != "artifacts"}, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    for kind in ("train", "holdout"):
        (args.output_dir / f"{kind}-reserve.json").write_text(
            json.dumps(result["artifacts"][kind], ensure_ascii=False, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )


if __name__ == "__main__":
    main()
