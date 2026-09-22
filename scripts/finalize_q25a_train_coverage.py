#!/usr/bin/env python3
"""Decide whether Q25a can preserve its teacher-label-only training contract."""

from __future__ import annotations

import argparse
import json
from collections import Counter
from pathlib import Path
from typing import Any

from prepare_q25_external_teacher_calibration import bind, sha256


SCHEMA = "sekirei.q25a-train-coverage-decision.v1"


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def finalize(preregistration: Path, labels: Path) -> dict[str, Any]:
    prereg = json.loads(preregistration.read_text(encoding="utf-8"))
    measured = json.loads(labels.read_text(encoding="utf-8"))
    require(
        prereg.get("schema") == "sekirei.q25a-external-label-execution-preregistration.v1"
        and prereg.get("status") == "frozen_before_any_external_or_self_label",
        "unexpected Q25a preregistration",
    )
    require(
        measured.get("schema") == "sekirei.q25a-external-label-measurements.v2"
        and measured.get("status") == "complete"
        and measured.get("kind") == "train"
        and measured.get("preregistration", {}).get("sha256") == sha256(preregistration),
        "unexpected Q25a train labels",
    )
    rows = measured.get("rows", [])
    expected = prereg["selection"]["train_parents"]
    require(len(rows) == expected, "Q25a did not process every frozen train parent")
    categories = prereg["selection"]["strata"]
    usable = [row for row in rows if row.get("label_status", "complete") == "complete" and row.get("ordinary_labeled_move_count", 0) >= 2]
    usable_by_category = Counter(row["category"] for row in usable)
    total_by_category = Counter(row["category"] for row in rows)
    required_per_category = prereg["selection"]["train_parents"] // len(categories)
    coverage_complete = (
        len(usable) == expected
        and all(total_by_category[category] == required_per_category for category in categories)
        and all(usable_by_category[category] == required_per_category for category in categories)
    )
    return {
        "schema": SCHEMA,
        "status": "candidate_training_authorized" if coverage_complete else "fail_insufficient_external_label_coverage",
        "diagnostic_only": True,
        "strength_claim": False,
        "candidate_trained": False,
        "candidate_adopted": False,
        "holdout_inspected": False,
        "holdout_labels_generated": False,
        "q20_authorized": False,
        "preregistration": bind(preregistration),
        "train_labels": bind(labels),
        "contract": {
            "single_factor": "teacher label source only",
            "required_train_parents": expected,
            "required_ordinary_external_scores_per_parent": 2,
            "required_per_stratum": required_per_category,
            "external_label_source": "A/A-stable fixed-depth free MultiPV",
            "forced_searchmoves": "not accepted because the pinned engine cannot enforce both forced move and depth",
        },
        "coverage": {
            "processed_parents": len(rows),
            "usable_parents": len(usable),
            "usable_rate": len(usable) / expected,
            "ordinary_labeled_move_histogram": dict(sorted(Counter(row.get("ordinary_labeled_move_count", 0) for row in rows).items())),
            "total_by_stratum": {category: total_by_category[category] for category in categories},
            "usable_by_stratum": {category: usable_by_category[category] for category in categories},
            "unsupported_moves_total": sum(row.get("unsupported_move_count", 0) for row in rows),
        },
        "interpretation": (
            "The external teacher may still be useful, but this pinned MultiPV=4 protocol cannot "
            "supply labels for every frozen self-candidate union. Training on only the usable subset "
            "would change sampling as well as the teacher, so it is not Q25a."
        ),
        "next_action": (
            "do not open the Q25a holdout or train a candidate; continue to Q26. A future external-label "
            "family requires a separately preregistered teacher/protocol that can label the full union."
        ),
        "finalizer": bind(Path(__file__).resolve()),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preregistration", type=Path, required=True)
    parser.add_argument("--train-labels", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = finalize(args.preregistration, args.train_labels)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"status": result["status"], **result["coverage"]}, ensure_ascii=False, sort_keys=True))
    return 0 if result["status"] == "candidate_training_authorized" else 1


if __name__ == "__main__":
    raise SystemExit(main())
