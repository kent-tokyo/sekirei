#!/usr/bin/env python3
"""Freeze the one Q25e candidate before opening its sealed hold-out."""
from __future__ import annotations

import argparse
import json
from pathlib import Path

from prepare_q25_external_teacher_calibration import bind


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--preregistration", type=Path, required=True)
    parser.add_argument("--coverage", type=Path, required=True)
    parser.add_argument("--train-pairs", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--metadata", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    coverage = json.loads(args.coverage.read_text(encoding="utf-8"))
    if coverage.get("status") != "candidate_training_authorized":
        parser.error("train coverage does not authorize a candidate")
    document = {
        "schema": "sekirei.q25e-candidate-freeze.v1",
        "status": "candidate_frozen_before_holdout_labeling",
        "diagnostic_only": True,
        "strength_claim": False,
        "candidate_adopted": False,
        "q20_authorized": False,
        "inputs": {
            "preregistration": bind(args.preregistration),
            "train_pair_coverage": bind(args.coverage),
            "train_pairs": bind(args.train_pairs),
            "candidate": bind(args.candidate),
            "training_metadata": bind(args.metadata),
        },
    }
    args.output.write_text(json.dumps(document, ensure_ascii=False, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"status": document["status"], "candidate_sha256": document["inputs"]["candidate"]["sha256"]}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
