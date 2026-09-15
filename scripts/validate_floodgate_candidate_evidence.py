#!/usr/bin/env python3
"""Validate the artifact references and conservative statuses in a candidate decision."""
from __future__ import annotations

import argparse
import json
from pathlib import Path

from validate_floodgate_candidate_decision import validate as validate_decision


def validate(document: dict, root: Path) -> list[str]:
    errors = validate_decision(document)
    evidence = document.get("evidence", {})
    if not isinstance(evidence, dict):
        return sorted(set(errors))
    manifest_releases = []
    for key in ("candidate_manifest", "pilot_manifest", "holdout_manifest", "holdout_regression_artifact", "resource_preflight_artifact", "holdout_alignment_artifact"):
        value = evidence.get(key)
        if not isinstance(value, str) or not value:
            errors.append(f"evidence.{key}")
            continue
        path = root / value
        try:
            referenced = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            errors.append(f"evidence.{key}.unreadable")
            continue
        if key.endswith("manifest") and referenced.get("schema") != "sekirei.release-manifest.v1":
            if key == "candidate_manifest" and referenced.get("schema") == "sekirei.floodgate-review-manifest.v1":
                provenance = referenced.get("provenance")
                if not isinstance(provenance, dict) or not isinstance(provenance.get("source_revision"), str):
                    errors.append("evidence.candidate_manifest.provenance")
                run_contract = referenced.get("run_contract")
                candidate_version = document.get("candidate", {}).get("version")
                if not isinstance(run_contract, dict) or run_contract.get("engine_version") != candidate_version:
                    errors.append("evidence.candidate_manifest.engine_version")
            else:
                errors.append(f"evidence.{key}.schema")
        if key.endswith("manifest") and isinstance(referenced.get("release"), str):
            manifest_releases.append(referenced["release"])
        if key == "holdout_regression_artifact":
            if referenced.get("schema") != "sekirei.floodgate-holdout-regression.v1":
                errors.append("evidence.holdout_regression_artifact.schema")
            if referenced.get("status") != "clean_for_this_diagnostic":
                errors.append("evidence.holdout_regression_artifact.status")
            if evidence.get("holdout_regression") != referenced.get("status"):
                errors.append("evidence.holdout_regression")
            if referenced.get("diagnostic_only") is not True:
                errors.append("evidence.holdout_regression_artifact.diagnostic_only")
        if key == "resource_preflight_artifact":
            if referenced.get("schema") != "sekirei.gate-resource-preflight.v1":
                errors.append("evidence.resource_preflight_artifact.schema")
            if referenced.get("verdict") != "refuse":
                errors.append("evidence.resource_preflight_artifact.verdict")
            if evidence.get("resource_preflight") != referenced.get("verdict"):
                errors.append("evidence.resource_preflight")
        if key == "holdout_alignment_artifact":
            if referenced.get("schema") != "sekirei.floodgate-candidate-holdout-alignment.v1":
                errors.append("evidence.holdout_alignment_artifact.schema")
            if referenced.get("status") != "not_aligned":
                errors.append("evidence.holdout_alignment_artifact.status")
            if "holdout_release_version_mismatch" not in referenced.get("reasons", []):
                errors.append("evidence.holdout_alignment_artifact.reason")
    candidate_version = document.get("candidate", {}).get("version")
    if manifest_releases and any(release != f"v{candidate_version}" for release in manifest_releases):
        if evidence.get("version_alignment") != "historical_evidence_not_candidate":
            errors.append("evidence.version_alignment")
        if document.get("status") != "inconclusive" or document.get("decision") != "keep_current":
            errors.append("version_mismatch_requires_keep_current")
    return sorted(set(errors))


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("decision", type=Path)
    parser.add_argument("--root", type=Path, default=Path.cwd())
    args = parser.parse_args(argv)
    document = json.loads(args.decision.read_text(encoding="utf-8"))
    errors = validate(document, args.root)
    if errors:
        print("invalid candidate evidence: " + ", ".join(errors))
        return 1
    print(f"candidate evidence valid: {args.decision}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
