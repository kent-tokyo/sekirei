#!/usr/bin/env python3
"""Bind Q21x/Q21y/Q21z decisions and cost measurements into one verdict."""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
from pathlib import Path
from statistics import median
from typing import Any


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def bind(path: Path) -> dict[str, str]:
    if not path.is_file():
        raise ValueError(f"missing artifact: {path}")
    return {"path": str(path), "sha256": sha256(path)}


def read(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def forward_results(path: Path) -> dict[str, Any]:
    rows = []
    with path.open(encoding="utf-8") as handle:
        for row in csv.reader(handle):
            if row and row[0] == "result":
                rows.append({"case": row[1], "median_ns": float(row[3]), "score": int(row[4])})
    if len(rows) != 3:
        raise ValueError(f"{path}: expected three forward results")
    return {"rows": rows, "median_ns": median(row["median_ns"] for row in rows)}


def profile_nodes(path: Path) -> dict[str, Any]:
    document = read(path)
    nodes = [row["teacher"]["nodes"] for row in document.get("rows", [])]
    if len(nodes) != 8:
        raise ValueError(f"{path}: expected eight same-time rows")
    return {"nodes": nodes, "median_nodes": median(nodes)}


def run(args: argparse.Namespace) -> dict[str, Any]:
    q21x = read(args.q21x_decision)
    q21y = read(args.q21y_decision)
    q21z = read(args.q21z_decision)
    if q21x.get("schema") != "sekirei.q21x-listwise-validation-decision.v1":
        raise ValueError("unexpected Q21x decision")
    if q21y.get("schema") != "sekirei.q21y-capacity-cv-decision.v1":
        raise ValueError("unexpected Q21y decision")
    if q21z.get("schema") != "sekirei.q21z-feature-cv-decision.v1":
        raise ValueError("unexpected Q21z decision")
    forward = {
        "base": forward_results(args.forward_base),
        "l1_384": forward_results(args.forward_l1),
        "l2_64": forward_results(args.forward_l2),
        "king_relative": forward_results(args.forward_feature),
    }
    base_ns = forward["base"]["median_ns"]
    for name, result in forward.items():
        result["ratio_to_base"] = result["median_ns"] / base_ns
    base_profile = profile_nodes(args.profile_base)
    feature_profile = profile_nodes(args.profile_feature)
    paired_ratios = [
        feature / base
        for base, feature in zip(base_profile["nodes"], feature_profile["nodes"], strict=True)
    ]
    search_cost = {
        "base": base_profile,
        "king_relative": feature_profile,
        "paired_node_ratio_median": median(paired_ratios),
        "interpretation": "same evaluator output at initialization; fixed-time node loss isolates feature maintenance/search cost, not strength",
    }
    no_candidate = q21x["status"] == "fail" and q21y["winner"] is None and q21z["winner"] is None
    document = {
        "schema": "sekirei.q21xyz-final-decision.v1",
        "status": "complete_no_candidate" if no_candidate else "candidate_requires_fresh_holdout",
        "diagnostic_only": True,
        "strength_claim": False,
        "candidate_adopted": False,
        "q20_authorized": False,
        "phases": {
            "q21x": {"status": q21x["status"], "static": q21x["static"], "same_time": q21x["same_time"]},
            "q21y": {"status": q21y["status"], "winner": q21y["winner"], "rows": q21y["rows"]},
            "q21z": {
                "status": q21z["status"],
                "winner": q21z["winner"],
                "baseline_metrics": q21z["baseline_metrics"],
                "feature_metrics": q21z["feature_metrics"],
                "mean_regret_reduction": q21z["mean_regret_reduction"],
                "conditions": q21z["conditions"],
            },
        },
        "forward_cost": forward,
        "same_time_search_cost": search_cost,
        "decision": (
            "retain L1=256/L2=32 and default features; do not combine failed factors or open another holdout"
            if no_candidate
            else "freeze the single passing factor before reserving a new score-blind holdout"
        ),
        "next_phase": "Q25 external-USI teacher calibration" if no_candidate else "fresh holdout confirmation",
        "artifacts": {
            name: bind(path)
            for name, path in {
                "q21x_decision": args.q21x_decision,
                "q21y_decision": args.q21y_decision,
                "q21z_decision": args.q21z_decision,
                "forward_base": args.forward_base,
                "forward_l1": args.forward_l1,
                "forward_l2": args.forward_l2,
                "forward_feature": args.forward_feature,
                "profile_base": args.profile_base,
                "profile_feature": args.profile_feature,
                "finalizer": Path(__file__).resolve(),
            }.items()
        },
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return document


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--q21x-decision", type=Path, required=True)
    parser.add_argument("--q21y-decision", type=Path, required=True)
    parser.add_argument("--q21z-decision", type=Path, required=True)
    parser.add_argument("--forward-base", type=Path, required=True)
    parser.add_argument("--forward-l1", type=Path, required=True)
    parser.add_argument("--forward-l2", type=Path, required=True)
    parser.add_argument("--forward-feature", type=Path, required=True)
    parser.add_argument("--profile-base", type=Path, required=True)
    parser.add_argument("--profile-feature", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        document = run(args)
    except (OSError, ValueError, KeyError, TypeError, json.JSONDecodeError) as error:
        parser.error(str(error))
    print(json.dumps({"status": document["status"], "decision": document["decision"], "next_phase": document["next_phase"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
