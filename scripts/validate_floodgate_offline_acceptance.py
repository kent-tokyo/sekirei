#!/usr/bin/env python3
"""Validate the offline-only Floodgate operational acceptance manifest."""
from __future__ import annotations

import argparse
import json
from pathlib import Path


REQUIRED_CASES = {
    "normal_game",
    "communication_loss",
    "explicit_stop",
    "record_initialization_failure",
    "supervisor_crash_and_stop",
}


def validate(document: object) -> list[str]:
    if not isinstance(document, dict):
        return ["document must be an object"]
    errors = []
    if document.get("schema") != "sekirei.floodgate-offline-acceptance.v1":
        errors.append("schema")
    if document.get("diagnostic_only") is not True:
        errors.append("diagnostic_only")
    cases = document.get("cases")
    if not isinstance(cases, list) or not cases:
        errors.append("cases")
    else:
        ids = {case.get("id") for case in cases if isinstance(case, dict)}
        if not REQUIRED_CASES.issubset(ids):
            errors.append("required cases")
        if any(
            not isinstance(case, dict)
            or case.get("status") != "passed"
            or not isinstance(case.get("fixture"), str)
            or not isinstance(case.get("assertions"), list)
            or not case["assertions"]
            for case in cases
        ):
            errors.append("case fields")
    if not isinstance(document.get("unverified"), list) or not document["unverified"]:
        errors.append("unverified")
    claims = document.get("claims", {})
    if claims.get("strength") != "not_permitted":
        errors.append("strength claim")
    if claims.get("operational_acceptance") != "offline_fixture_only":
        errors.append("operational acceptance")
    if claims.get("release_approval") != "not_granted":
        errors.append("release approval")
    return errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    args = parser.parse_args()
    errors = validate(json.loads(args.manifest.read_text(encoding="utf-8")))
    if errors:
        print("invalid offline acceptance manifest: " + ", ".join(errors))
        return 1
    print("offline acceptance manifest valid")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
