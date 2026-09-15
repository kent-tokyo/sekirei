#!/usr/bin/env python3
"""Assess whether a hold-out artifact is evidence for the current candidate."""
from __future__ import annotations

import argparse
import json
from pathlib import Path


SCHEMA = "sekirei.floodgate-candidate-holdout-alignment.v1"


def assess(candidate_manifest: dict, holdout_manifest: dict, regression: dict) -> dict:
    reasons = []
    candidate_contract = candidate_manifest.get("run_contract", {})
    candidate_version = candidate_contract.get("engine_version")
    holdout_release = holdout_manifest.get("release")
    if candidate_manifest.get("schema") != "sekirei.floodgate-review-manifest.v1":
        reasons.append("candidate_manifest_schema")
    if not isinstance(candidate_version, str) or not candidate_version:
        reasons.append("candidate_engine_version_missing")
    if holdout_manifest.get("schema") != "sekirei.release-manifest.v1":
        reasons.append("holdout_manifest_schema")
    if isinstance(candidate_version, str) and holdout_release != f"v{candidate_version}":
        reasons.append("holdout_release_version_mismatch")
    if regression.get("schema") != "sekirei.floodgate-holdout-regression.v1":
        reasons.append("regression_schema")
    if regression.get("status") != "clean_for_this_diagnostic":
        reasons.append("regression_not_clean")
    status = "aligned" if not reasons else "not_aligned"
    return {
        "schema": SCHEMA,
        "diagnostic_only": True,
        "status": status,
        "reasons": reasons,
        "candidate_version": candidate_version,
        "holdout_release": holdout_release,
        "regression_status": regression.get("status"),
        "claims": {
            "strength": "not_permitted",
            "candidate_adoption": "not_established",
            "holdout_scope": "artifact_alignment_only",
        },
    }


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("candidate_manifest", type=Path)
    parser.add_argument("holdout_manifest", type=Path)
    parser.add_argument("regression", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    result = assess(
        json.loads(args.candidate_manifest.read_text(encoding="utf-8")),
        json.loads(args.holdout_manifest.read_text(encoding="utf-8")),
        json.loads(args.regression.read_text(encoding="utf-8")),
    )
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"hold-out alignment: {result['status']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
