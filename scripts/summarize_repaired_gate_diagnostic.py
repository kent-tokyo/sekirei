#!/usr/bin/env python3
"""State the bounded conclusion of a repaired post-gate diagnostic.

This deliberately records an ``unknown`` conclusion when a result cannot be
compared at the same completed depth.  It is a guard against converting one
post-hoc game-tree branch into a training label or a strength claim.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path


def artifact(path: Path) -> dict[str, str]:
    return {"path": str(path), "sha256": hashlib.sha256(path.read_bytes()).hexdigest()}


def load(path: Path) -> dict:
    document = json.loads(path.read_text(encoding="utf-8"))
    if document.get("diagnostic_only") is not True:
        raise ValueError(f"{path}: must be diagnostic-only")
    return document


def build(evaluator_100k: Path, evaluator_1m: Path, candidate_1m: Path,
          nmp: Path, lmr: Path) -> dict:
    shallow, deep, candidate, nmp_document, lmr_document = map(
        load, (evaluator_100k, evaluator_1m, candidate_1m, nmp, lmr)
    )
    shallow_summary, deep_summary = shallow.get("summary", {}), deep.get("summary", {})
    candidate_rows = candidate.get("results", [])
    if len(candidate_rows) != 1:
        raise ValueError("candidate 1M diagnostic must contain exactly one selected position")
    root = candidate_rows[0].get("comparison", {})
    candidate_evaluator_supported = (
        shallow_summary.get("sign_mismatches") not in (0, None)
        or deep_summary.get("bestmove_changes") not in (0, None)
    )
    pruning_decisive = any(
        document.get("summary", {}).get("same_completed_depth") == 1
        and document.get("summary", {}).get("bestmove_changes")
        for document in (nmp_document, lmr_document)
    )
    return {
        "schema": "sekirei.repaired-gate-diagnostic-summary.v1",
        "diagnostic_only": True,
        "strength_claim": "not_permitted",
        "inputs": {
            "evaluator_100k": artifact(evaluator_100k),
            "evaluator_1m": artifact(evaluator_1m),
            "candidate_1m": artifact(candidate_1m),
            "nmp_1m": artifact(nmp),
            "lmr_1m": artifact(lmr),
        },
        "observed": {
            "evaluator_100k": shallow_summary,
            "evaluator_1m": deep_summary,
            "candidate_free_vs_observed_root": root,
            "nmp_1m": nmp_document.get("summary", {}),
            "lmr_1m": lmr_document.get("summary", {}),
        },
        "conclusion": {
            "candidate_vs_baseline_nnue_explains_selected_gate_loss": (
                "supported" if candidate_evaluator_supported else "not_supported_on_this_bounded_sample"
            ),
            "nmp_or_lmr_explains_selected_gate_loss": (
                "supported" if pruning_decisive else "not_supported_at_matched_depth"
            ),
            "observed_candidate_move": (
                "free_search_disagrees_at_same_completed_depth"
                if root.get("same_completed_depth") is True
                and root.get("unrestricted_bestmove_matches_played") is False
                else "not_comparable"
            ),
            "next_action": (
                "do_not_adopt_or_train_from_this_post_hoc sample; freeze a new independent "
                "fixed-depth root-move corpus before any one-factor candidate"
            ),
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--evaluator-100k", type=Path, required=True)
    parser.add_argument("--evaluator-1m", type=Path, required=True)
    parser.add_argument("--candidate-1m", type=Path, required=True)
    parser.add_argument("--nmp", type=Path, required=True)
    parser.add_argument("--lmr", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        document = build(args.evaluator_100k, args.evaluator_1m, args.candidate_1m, args.nmp, args.lmr)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {args.output}: {document['conclusion']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
