#!/usr/bin/env python3
"""Audit score-free tactical coverage before a new self-teacher family.

This is deliberately an audit, not a selector: it binds a pre-existing parent
reserve and records only legal-move facts.  No NNUE, search, teacher score, or
candidate checkpoint is loaded.  A later Q28 family may use one documented
source-distribution factor, but must create a fresh score-blind boundary.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter, defaultdict
from pathlib import Path
import subprocess
from typing import Any


SCHEMA = "sekirei.q28-source-distribution-audit.v1"
REQUIRED_FACTS = {
    "legal_moves",
    "in_check",
    "mate_in_one",
    "capture_moves",
    "checking_moves",
    "forcing_moves",
}


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def bind(path: Path) -> dict[str, str]:
    return {"path": str(path), "sha256": sha256(path)}


def parse_fact_line(line: str) -> dict[str, int | bool]:
    values = dict(field.split("=", 1) for field in line.split("\t") if "=" in field)
    if set(values) != REQUIRED_FACTS:
        raise ValueError(f"unexpected rule probe fields: {line!r}")
    facts: dict[str, int | bool] = {}
    for key in ("legal_moves", "capture_moves", "checking_moves", "forcing_moves"):
        try:
            facts[key] = int(values[key])
        except ValueError as error:
            raise ValueError(f"invalid integer {key}: {values[key]!r}") from error
        if facts[key] < 0:
            raise ValueError(f"negative {key}")
    for key in ("in_check", "mate_in_one"):
        if values[key] not in {"true", "false"}:
            raise ValueError(f"invalid boolean {key}: {values[key]!r}")
        facts[key] = values[key] == "true"
    if facts["forcing_moves"] > facts["legal_moves"]:
        raise ValueError("forcing move count exceeds legal move count")
    if facts["checking_moves"] > facts["forcing_moves"] or facts["capture_moves"] > facts["forcing_moves"]:
        raise ValueError("component tactical count exceeds forcing move count")
    return facts


def tactical_class(facts: dict[str, int | bool]) -> str:
    """A disjoint class chosen entirely from legal-move facts."""
    if bool(facts["in_check"]):
        return "evasion"
    if int(facts["checking_moves"]) > 0:
        return "checking_resource"
    if int(facts["capture_moves"]) > 0:
        return "capture_resource"
    return "quiet"


def rule_facts(probe: Path, positions: list[dict[str, Any]]) -> list[dict[str, int | bool]]:
    completed = subprocess.run(
        [str(probe)],
        input="\n".join(str(row["sfen"]) for row in positions) + "\n",
        capture_output=True,
        check=False,
        text=True,
    )
    if completed.returncode != 0:
        raise ValueError(completed.stderr.strip() or "rule probe failed")
    lines = completed.stdout.splitlines()
    if len(lines) != len(positions):
        raise ValueError("rule probe returned a different row count")
    return [parse_fact_line(line) for line in lines]


def summarize(positions: list[dict[str, Any]], facts: list[dict[str, int | bool]]) -> dict[str, Any]:
    if len(positions) != len(facts):
        raise ValueError("position/fact count mismatch")
    classes = Counter(tactical_class(row) for row in facts)
    by_category: dict[str, Counter[str]] = defaultdict(Counter)
    metric_values: dict[str, list[int]] = defaultdict(list)
    for position, row in zip(positions, facts):
        category = str(position["category"])
        by_category[category][tactical_class(row)] += 1
        for key in ("legal_moves", "capture_moves", "checking_moves", "forcing_moves"):
            metric_values[key].append(int(row[key]))
    return {
        "parents": len(positions),
        "tactical_class_counts": dict(sorted(classes.items())),
        "tactical_class_fraction": {
            key: count / len(positions) if positions else 0.0
            for key, count in sorted(classes.items())
        },
        "by_phase_material": {
            category: dict(sorted(counts.items()))
            for category, counts in sorted(by_category.items())
        },
        "move_count_summary": {
            key: {
                "min": min(values) if values else 0,
                "max": max(values) if values else 0,
                "mean": sum(values) / len(values) if values else 0.0,
            }
            for key, values in sorted(metric_values.items())
        },
    }


def audit(args: argparse.Namespace) -> dict[str, Any]:
    reserve = json.loads(args.reserve.read_text(encoding="utf-8"))
    positions = reserve.get("positions")
    if not isinstance(positions, list) or not positions:
        raise ValueError("reserve has no positions")
    if any(not isinstance(row, dict) or not isinstance(row.get("sfen"), str) for row in positions):
        raise ValueError("reserve has invalid SFEN rows")
    facts = rule_facts(args.rule_probe, positions)
    return {
        "schema": SCHEMA,
        "status": "complete",
        "diagnostic_only": True,
        "selection_used_scores": False,
        "strength_claim": False,
        "contract": {
            "input_reserve_is_preexisting": True,
            "forbidden_inputs": ["NNUE", "search", "teacher_score", "candidate_checkpoint"],
            "tactical_classes": ["evasion", "checking_resource", "capture_resource", "quiet"],
        },
        "inputs": {"reserve": bind(args.reserve), "rule_probe": bind(args.rule_probe)},
        "summary": summarize(positions, facts),
        "rows": [
            {
                "id": position["id"],
                "category": position["category"],
                "source_key": position.get("source", {}).get("source_key"),
                "tactical_class": tactical_class(row),
                "rule_facts": row,
            }
            for position, row in zip(positions, facts)
        ],
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reserve", type=Path, required=True)
    parser.add_argument("--rule-probe", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        parser.error("output already exists; use a new audit path")
    result = audit(args)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
