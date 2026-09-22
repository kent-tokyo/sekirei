#!/usr/bin/env python3
"""Freeze a larger, score-free Q29 parent boundary without changing its recipe."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

import prepare_q21w_coverage as q21w
from prepare_q25_external_teacher_calibration import bind
import audit_q28_source_distribution as rules
from scan_q29_coverage_pool import SCHEMA as POOL_SCHEMA


FAMILY_SCHEMA = "sekirei.q29-coverage-family-preregistration.v1"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def identity(row: dict[str, Any]) -> str:
    return q21w.q21h.symmetry_key(str(row["sfen"]))


def rank(seed: int, category: str, purpose: str, row: dict[str, Any]) -> int:
    source = row["source"]
    return q21w.stable_rank(seed, f"q29-{category}-{purpose}", f"{source['source_key']}\0{row['sfen']}")


def take(
    rows: list[dict[str, Any]], *, category: str, purpose: str, limit: int, seed: int,
    used_sources: set[str], used_identities: set[str],
) -> list[dict[str, Any]]:
    selected: list[dict[str, Any]] = []
    for row in sorted(rows, key=lambda item: rank(seed, category, purpose, item)):
        source_key = str(row["source"]["source_key"])
        group = identity(row)
        if source_key in used_sources or group in used_identities:
            continue
        selected.append(row)
        used_sources.add(source_key)
        used_identities.add(group)
        if len(selected) == limit:
            return selected
    raise ValueError(f"{category}: insufficient score-free Q29 candidates for {purpose}")


def corpus(kind: str, rows: list[dict[str, Any]]) -> dict[str, Any]:
    return {
        "schema": f"sekirei.q29-{kind}-reserve.v1",
        "diagnostic_only": True,
        "strength_claim": False,
        "selection_used_scores": False,
        "positions": [
            {
                "id": f"q29-{kind}-{index:03d}", "category": row["category"],
                "initial_sfen": row["sfen"], "history_before_usi": [], "sfen": row["sfen"],
                "source": row["source"],
            }
            for index, row in enumerate(sorted(rows, key=lambda item: (item["category"], item["source"]["source_key"])), 1)
        ],
    }


def excluded(reserves: list[Path]) -> tuple[set[str], set[str]]:
    sources: set[str] = set()
    groups: set[str] = set()
    for reserve in reserves:
        document = json.loads(reserve.read_text(encoding="utf-8"))
        positions = document.get("positions")
        require(isinstance(positions, list), f"{reserve}: positions missing")
        for row in positions:
            source = row.get("source", {})
            source_key = source.get("source_key")
            group = source.get("derived_group")
            require(isinstance(source_key, str) and isinstance(group, str), f"{reserve}: invalid source")
            sources.add(source_key)
            groups.add(group)
    return sources, groups


def run(args: argparse.Namespace) -> dict[str, Any]:
    pool = json.loads(args.pool_scan.read_text(encoding="utf-8"))
    require(pool.get("schema") == POOL_SCHEMA and pool.get("status") == "complete", "invalid Q29 pool")
    facts = rules.rule_facts(args.rule_probe, pool["rows"])
    excluded_sources, excluded_groups = excluded(args.exclude_reserve)
    grouped: dict[str, list[dict[str, Any]]] = {category: [] for category in q21w.STRATA}
    for row, fact in zip(pool["rows"], facts):
        require(row.get("category") in grouped, "unexpected Q29 category")
        source = row["source"]
        if (
            int(fact["legal_moves"]) >= 2
            and not bool(fact["mate_in_one"])
            and source["source_key"] not in excluded_sources
            and source["derived_group"] not in excluded_groups
        ):
            grouped[row["category"]].append(row)
    used_sources: set[str] = set()
    used_identities: set[str] = set()
    train_rows: list[dict[str, Any]] = []
    holdout_rows: list[dict[str, Any]] = []
    for category in q21w.STRATA:
        train_rows.extend(take(grouped[category], category=category, purpose="train", limit=16, seed=args.seed, used_sources=used_sources, used_identities=used_identities))
        holdout_rows.extend(take(grouped[category], category=category, purpose="holdout", limit=4, seed=args.seed, used_sources=used_sources, used_identities=used_identities))
    require(len(train_rows) == 144 and len(holdout_rows) == 36, "Q29 boundary count drift")
    return {
        "schema": FAMILY_SCHEMA,
        "status": "frozen_before_any_teacher_label",
        "diagnostic_only": True,
        "strength_claim": False,
        "single_factor": "independent parent coverage 72/18 to 144/36; teacher, architecture, features, optimizer, seed, epochs, pair form, and screen conditions stay fixed",
        "inputs": {"pool": bind(args.pool_scan), "selector": bind(Path(__file__).resolve())},
        "source_selection": {
            "used_scores": False, "seed": args.seed, "train_per_stratum": 16, "holdout_per_stratum": 4,
            "rule_only_eligibility": "legal_moves >= 2 and mate_in_one == false",
            "rule_probe": bind(args.rule_probe), "excluded_reserves": [bind(path) for path in args.exclude_reserve],
            "eligible_sources_per_stratum": {category: len({row["source"]["source_key"] for row in rows}) for category, rows in grouped.items()},
        },
        "artifacts": {"train": corpus("train", train_rows), "holdout": corpus("holdout", holdout_rows)},
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pool-scan", type=Path, required=True)
    parser.add_argument("--rule-probe", type=Path, required=True)
    parser.add_argument("--exclude-reserve", type=Path, action="append", default=[])
    parser.add_argument("--seed", type=int, default=2902)
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
