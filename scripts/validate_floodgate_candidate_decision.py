#!/usr/bin/env python3
"""Validate the conservative FG5-e candidate decision boundary."""
from __future__ import annotations

import argparse
import json
from pathlib import Path


def validate(document: object) -> list[str]:
    if not isinstance(document, dict):
        return ["document must be an object"]
    errors = []
    if document.get("schema") != "sekirei.floodgate-candidate-decision.v1":
        errors.append("schema")
    if document.get("diagnostic_only") is not True:
        errors.append("diagnostic_only")
    if document.get("status") not in {"inconclusive", "not_adopted"}:
        errors.append("status")
    if document.get("decision") not in {"keep_current", "not_adopted"}:
        errors.append("decision")
    claims = document.get("claims", {})
    if claims.get("strength") != "not_permitted":
        errors.append("strength claim")
    if claims.get("release_approval") != "not_granted":
        errors.append("release approval")
    if claims.get("competitor_superiority") != "not_established":
        errors.append("competitor claim")
    candidate = document.get("candidate")
    if not isinstance(candidate, dict) or not isinstance(candidate.get("version"), str) or not candidate["version"]:
        errors.append("candidate.version")
    elif candidate.get("formal_adoption") is not False:
        errors.append("candidate.formal_adoption")
    evidence = document.get("evidence")
    if not isinstance(evidence, dict):
        errors.append("evidence")
    else:
        for key in ("candidate_manifest", "pilot_manifest", "holdout_manifest", "gate_status", "holdout_regression", "nmp_ablation", "resource_preflight", "version_alignment", "holdout_alignment_artifact"):
            if not isinstance(evidence.get(key), str) or not evidence[key]:
                errors.append(f"evidence.{key}")
    if not isinstance(document.get("reasons"), list) or not document["reasons"]:
        errors.append("reasons")
    if not isinstance(document.get("next_action"), str) or not document["next_action"]:
        errors.append("next_action")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("decision", type=Path)
    args = parser.parse_args()
    errors = validate(json.loads(args.decision.read_text(encoding="utf-8")))
    if errors:
        print("invalid candidate decision: " + ", ".join(errors))
        return 1
    print("candidate decision valid")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
