#!/usr/bin/env python3
"""Freeze a score-blind retry for Q21n's sole unlabeled timeout parent."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def bind(path: Path) -> dict[str, str]:
    return {"path": str(path), "sha256": sha256(path)}


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def prepare(args: argparse.Namespace) -> tuple[dict[str, Any], dict[str, Any]]:
    prereg = json.loads(args.preregistration.read_text(encoding="utf-8"))
    corpus = json.loads(args.corpus.read_text(encoding="utf-8"))
    initial = json.loads(args.initial_deep.read_text(encoding="utf-8"))
    require(
        prereg.get("schema") == "sekirei.q21n-teacher-depth-preregistration.v1"
        and prereg.get("status") == "frozen_before_deep_labels",
        "unexpected Q21n preregistration",
    )
    require(prereg["inputs"]["corpus"]["sha256"] == sha256(args.corpus), "corpus SHA mismatch")
    require(
        initial.get("schema") == "sekirei.root-rank-teacher-corpus.v1"
        and initial.get("contract", {}).get("depth") == prereg["deep_contract"]["depth"],
        "unexpected initial deep artifact",
    )
    incomplete = [
        row for row in initial.get("rows", [])
        if row.get("candidate_prefix_complete") is not True
        or row.get("complete_legal_root_set") is not True
    ]
    require(len(initial.get("rows", [])) == 18 and len(incomplete) == 1, "retry requires exactly one incomplete parent")
    failed = incomplete[0]
    result = failed.get("teacher_root", {})
    require(
        result.get("completion") == "timeout"
        and not result.get("root_candidates")
        and result.get("score_cp") is None,
        "retry parent must be an unlabeled timeout, not a measured disagreement",
    )
    position = next(
        (row for row in corpus.get("positions", []) if row.get("id") == failed.get("id")),
        None,
    )
    require(position is not None, "timeout parent is absent from the frozen corpus")
    retry_corpus = {
        "schema": "sekirei.q21n-timeout-retry-corpus.v1",
        "diagnostic_only": True,
        "strength_claim": False,
        "selection": {
            "rule": "the sole parent with no score or ranking because the frozen 300s attempt timed out",
            "score_blind": True,
            "positions": 1,
        },
        "positions": [position],
    }
    args.retry_corpus.parent.mkdir(parents=True, exist_ok=True)
    args.retry_corpus.write_text(
        json.dumps(retry_corpus, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    amendment = {
        "schema": "sekirei.q21n-timeout-retry-amendment.v1",
        "status": "frozen_before_retry_label",
        "diagnostic_only": True,
        "strength_claim": False,
        "reason": "one parent returned timeout without a score or root-candidate ranking",
        "inputs": {
            "preregistration": bind(args.preregistration),
            "full_corpus": bind(args.corpus),
            "initial_deep": bind(args.initial_deep),
            "retry_corpus": bind(args.retry_corpus),
        },
        "retry": {
            "position_id": failed["id"],
            "only_incomplete_parent": True,
            "observed_label_before_retry": False,
            "depth": prereg["deep_contract"]["depth"],
            "root_candidate_limit": prereg["deep_contract"]["root_candidate_limit"],
            "threads": 1,
            "spec_top_n": 0,
            "timeout_seconds": 900,
            "binary_sha256": prereg["inputs"]["binary"]["sha256"],
            "weights_sha256": prereg["inputs"]["weights"]["sha256"],
            "nnue_output": "residual-material",
        },
        "decision_rule_unchanged": True,
    }
    return amendment, retry_corpus


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preregistration", type=Path, required=True)
    parser.add_argument("--corpus", type=Path, required=True)
    parser.add_argument("--initial-deep", type=Path, required=True)
    parser.add_argument("--retry-corpus", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        amendment, _ = prepare(args)
    except (OSError, ValueError, KeyError, StopIteration, json.JSONDecodeError) as error:
        parser.error(str(error))
    args.output.write_text(
        json.dumps(amendment, ensure_ascii=False, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    print(json.dumps({"status": amendment["status"], **amendment["retry"]}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
