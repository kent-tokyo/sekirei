#!/usr/bin/env python3
"""Apply the pre-registered Q21i ranking-loss and major-blunder screen."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def diagnostic(path: Path) -> dict:
    document = json.loads(path.read_text(encoding="utf-8"))
    if document.get("schema") != "sekirei.root-rank-pair-audit.v1":
        raise ValueError(f"{path}: unsupported audit schema")
    value = document.get("model_diagnostic")
    if not isinstance(value, dict) or not isinstance(value.get("parent_diagnostics"), list):
        raise ValueError(f"{path}: parent ranking diagnostics are required")
    return value


def evaluate(baseline: dict, candidate: dict) -> dict:
    baseline_parents = {row["parent_id"] for row in baseline["parent_diagnostics"]}
    candidate_parents = {row["parent_id"] for row in candidate["parent_diagnostics"]}
    if baseline_parents != candidate_parents or not baseline_parents:
        raise ValueError("baseline/candidate parent sets differ or are empty")
    baseline_loss = float(baseline["mean_parent_rank_loss_cp"])
    candidate_loss = float(candidate["mean_parent_rank_loss_cp"])
    improvement = (baseline_loss - candidate_loss) / baseline_loss if baseline_loss > 0.0 else 0.0
    major_nonincrease = int(candidate["major_blunders_ge_300cp"]) <= int(baseline["major_blunders_ge_300cp"])
    passed = improvement >= 0.10 and major_nonincrease
    return {
        "schema": "sekirei.q21i-imitation-screen.v1",
        "strength_claim": False,
        "parents": len(baseline_parents),
        "criteria": {"minimum_rank_loss_reduction": 0.10, "major_blunders_must_not_increase": True},
        "baseline": {
            "mean_parent_rank_loss_cp": baseline_loss,
            "major_blunders_ge_300cp": baseline["major_blunders_ge_300cp"],
        },
        "candidate": {
            "mean_parent_rank_loss_cp": candidate_loss,
            "major_blunders_ge_300cp": candidate["major_blunders_ge_300cp"],
        },
        "rank_loss_reduction": improvement,
        "checks": {"rank_loss_reduction_pass": improvement >= 0.10, "major_blunders_nonincrease": major_nonincrease},
        "status": "screen_pass" if passed else "screen_fail",
        "next_action": "run_32_game_development_match" if passed else "reject_or_use_one_preregistered_contingency_recipe",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        result = evaluate(diagnostic(args.baseline), diagnostic(args.candidate))
    except (OSError, ValueError, json.JSONDecodeError) as error:
        parser.error(str(error))
    result["inputs"] = {
        "baseline": {"path": str(args.baseline), "sha256": sha256(args.baseline)},
        "candidate": {"path": str(args.candidate), "sha256": sha256(args.candidate)},
    }
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"status": result["status"], "rank_loss_reduction": result["rank_loss_reduction"]}, sort_keys=True))
    return 0 if result["status"] == "screen_pass" else 1


if __name__ == "__main__":
    raise SystemExit(main())
