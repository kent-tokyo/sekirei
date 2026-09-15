#!/usr/bin/env python3
"""Apply a predeclared non-cherry-picking rule to root-ranking diagnostics."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def model(path: Path) -> dict:
    document = json.loads(path.read_text(encoding="utf-8"))
    if document.get("schema") != "sekirei.root-rank-pair-audit.v1" or document.get("diagnostic_only") is not True:
        raise ValueError(f"{path}: unsupported ranking audit")
    result = document.get("model_diagnostic")
    if not isinstance(result, dict):
        raise ValueError(f"{path}: missing model diagnostic")
    for key in ("pairs_scored", "teacher_preferred_ordered_pairs", "teacher_preferred_ordering_rate",
                "mean_pairwise_logistic_loss", "mean_parent_oriented_margin_cp"):
        if not isinstance(result.get(key), (int, float)):
            raise ValueError(f"{path}: invalid {key}")
    return result


def evaluate(baseline: dict, candidate: dict, expected_pairs: int) -> dict:
    if baseline["pairs_scored"] != expected_pairs or candidate["pairs_scored"] != expected_pairs:
        raise ValueError("audit pair counts do not match frozen stable pair set")
    ordering_nonworse = candidate["teacher_preferred_ordered_pairs"] >= baseline["teacher_preferred_ordered_pairs"]
    loss_nonworse = candidate["mean_pairwise_logistic_loss"] <= baseline["mean_pairwise_logistic_loss"]
    margin_nonworse = candidate["mean_parent_oriented_margin_cp"] >= baseline["mean_parent_oriented_margin_cp"]
    passed = ordering_nonworse and loss_nonworse and margin_nonworse
    return {
        "schema": "sekirei.root-ranking-candidate-decision.v1",
        "diagnostic_only": True,
        "strength_claim": "not_permitted",
        "criteria": {
            "ordering": "candidate ordered-pair count >= baseline",
            "logistic_loss": "candidate mean loss <= baseline",
            "margin": "candidate mean parent-oriented margin >= baseline",
            "all_required": True,
        },
        "baseline": baseline,
        "candidate": candidate,
        "checks": {
            "ordering_nonworse": ordering_nonworse,
            "loss_nonworse": loss_nonworse,
            "margin_nonworse": margin_nonworse,
        },
        "status": "eligible_for_next_diagnostic_only" if passed else "rejected_no_cherry_picking",
        "candidate_adoption": "not_established",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--stable-manifest", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    stable = json.loads(args.stable_manifest.read_text(encoding="utf-8"))
    expected_pairs = stable.get("retained_pairs")
    if not isinstance(expected_pairs, int) or expected_pairs <= 0:
        parser.error("stable manifest lacks retained_pairs")
    try:
        document = evaluate(model(args.baseline), model(args.candidate), expected_pairs)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        parser.error(str(error))
    document["inputs"] = {
        "baseline": {"path": str(args.baseline), "sha256": sha256(args.baseline)},
        "candidate": {"path": str(args.candidate), "sha256": sha256(args.candidate)},
        "stable_manifest": {"path": str(args.stable_manifest), "sha256": sha256(args.stable_manifest)},
    }
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {document['status']}")
    return 0 if document["status"] == "eligible_for_next_diagnostic_only" else 1


if __name__ == "__main__":
    raise SystemExit(main())
