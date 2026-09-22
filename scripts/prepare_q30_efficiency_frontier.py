#!/usr/bin/env python3
"""Freeze Q30's score-blind 72/18 capacity-efficiency boundary."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

import audit_q28_source_distribution as rules
import prepare_q21w_coverage as q21w
import prepare_q29_coverage_scale as q29
from prepare_q25_external_teacher_calibration import bind
from scan_q30_efficiency_pool import SCHEMA as POOL_SCHEMA


FAMILY_SCHEMA = "sekirei.q30-efficiency-family-preregistration.v1"
TRAIN_PER_STRATUM = 8
HOLDOUT_PER_STRATUM = 2


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def rank(seed: int, category: str, purpose: str, row: dict[str, Any]) -> int:
    source = row["source"]
    return q21w.stable_rank(seed, f"q30-{category}-{purpose}", f"{source['source_key']}\0{row['sfen']}")


def take(
    rows: list[dict[str, Any]], *, category: str, purpose: str, limit: int, seed: int,
    used_sources: set[str], used_groups: set[str],
) -> list[dict[str, Any]]:
    selected = []
    for row in sorted(rows, key=lambda item: rank(seed, category, purpose, item)):
        source = row["source"]
        source_key = str(source["source_key"])
        group = str(source["derived_group"])
        if source_key in used_sources or group in used_groups:
            continue
        selected.append(row)
        used_sources.add(source_key)
        used_groups.add(group)
        if len(selected) == limit:
            return selected
    raise ValueError(f"{category}: insufficient Q30 score-blind eligible positions for {purpose}")


def corpus(kind: str, rows: list[dict[str, Any]]) -> dict[str, Any]:
    return {
        "schema": f"sekirei.q30-{kind}-reserve.v1",
        "diagnostic_only": True,
        "strength_claim": False,
        "selection_used_scores": False,
        "positions": [
            {
                "id": f"q30-{kind}-{index:03d}",
                "category": row["category"],
                "initial_sfen": row["sfen"],
                "history_before_usi": [],
                "sfen": row["sfen"],
                "source": row["source"],
            }
            for index, row in enumerate(sorted(rows, key=lambda item: (item["category"], item["source"]["source_key"])), 1)
        ],
    }


def run(args: argparse.Namespace) -> dict[str, Any]:
    pool = json.loads(args.pool_scan.read_text(encoding="utf-8"))
    require(pool.get("schema") == POOL_SCHEMA and pool.get("status") == "complete", "invalid Q30 pool")
    facts = rules.rule_facts(args.rule_probe, pool["rows"])
    excluded_sources, excluded_groups = q29.excluded(args.exclude_reserve)
    grouped: dict[str, list[dict[str, Any]]] = {category: [] for category in q21w.STRATA}
    for row, fact in zip(pool["rows"], facts):
        source = row["source"]
        require(row.get("category") in grouped, "unexpected Q30 category")
        if (
            int(fact["legal_moves"]) >= 2
            and not bool(fact["mate_in_one"])
            and source["source_key"] not in excluded_sources
            and source["derived_group"] not in excluded_groups
        ):
            grouped[row["category"]].append(row)
    used_sources: set[str] = set()
    used_groups: set[str] = set()
    train_rows: list[dict[str, Any]] = []
    holdout_rows: list[dict[str, Any]] = []
    for category in q21w.STRATA:
        train_rows.extend(take(grouped[category], category=category, purpose="train", limit=TRAIN_PER_STRATUM, seed=args.seed, used_sources=used_sources, used_groups=used_groups))
        holdout_rows.extend(take(grouped[category], category=category, purpose="holdout", limit=HOLDOUT_PER_STRATUM, seed=args.seed, used_sources=used_sources, used_groups=used_groups))
    require(len(train_rows) == 72 and len(holdout_rows) == 18, "Q30 boundary count drift")
    return {
        "schema": FAMILY_SCHEMA,
        "status": "frozen_before_any_teacher_label",
        "diagnostic_only": True,
        "strength_claim": False,
        "single_factor": "NNUE capacity 256/32 versus 128/16; teacher, labels, feature set, optimizer, seed, epochs, pair form, and screen conditions stay fixed",
        "inputs": {"pool": bind(args.pool_scan), "selector": bind(Path(__file__).resolve())},
        "source_selection": {
            "used_scores": False,
            "seed": args.seed,
            "train_per_stratum": TRAIN_PER_STRATUM,
            "holdout_per_stratum": HOLDOUT_PER_STRATUM,
            "rule_only_eligibility": "legal_moves >= 2 and mate_in_one == false",
            "rule_probe": bind(args.rule_probe),
            "excluded_reserves": [bind(path) for path in args.exclude_reserve],
            "eligible_sources_per_stratum": {category: len({row["source"]["source_key"] for row in rows}) for category, rows in grouped.items()},
        },
        "artifacts": {"train": corpus("train", train_rows), "holdout": corpus("holdout", holdout_rows)},
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pool-scan", type=Path, required=True)
    parser.add_argument("--rule-probe", type=Path, required=True)
    parser.add_argument("--exclude-reserve", type=Path, action="append", default=[])
    parser.add_argument("--seed", type=int, default=3002)
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    if args.output_dir.exists():
        parser.error("output directory already exists; selection boundary is immutable")
    try:
        result = run(args)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output_dir.mkdir(parents=True)
    (args.output_dir / "preregistration.json").write_text(json.dumps({key: value for key, value in result.items() if key != "artifacts"}, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    for kind in ("train", "holdout"):
        (args.output_dir / f"{kind}-reserve.json").write_text(json.dumps(result["artifacts"][kind], ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
