#!/usr/bin/env python3
"""Validate a completed Q21i NNUE cost-profile artifact set."""

import argparse
import hashlib
import json
from pathlib import Path


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("artifact_dir", type=Path)
    args = parser.parse_args()
    prereg_path = args.artifact_dir / "preregistration.json"
    profile_path = args.artifact_dir / "profile.json"
    summary_path = args.artifact_dir / "summary.json"
    errors = []
    try:
        prereg = json.loads(prereg_path.read_text(encoding="utf-8"))
        profile = json.loads(profile_path.read_text(encoding="utf-8"))
        summary = json.loads(summary_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        print(json.dumps({"valid": False, "errors": [str(error)]}, sort_keys=True))
        return 1
    if profile.get("preregistration_sha256") != sha256(prereg_path):
        errors.append("preregistration hash mismatch")
    if summary.get("profile_sha256") != sha256(profile_path):
        errors.append("profile hash mismatch")
    ids = prereg.get("position_ids", [])
    if len(ids) != 9 or len(set(ids)) != 9:
        errors.append("expected nine unique preregistered positions")
    strata = {
        (item.get("phase"), item.get("material_band")) for item in prereg.get("strata", [])
    }
    if len(strata) != 9:
        errors.append("expected all nine phase/material strata")
    rows = profile.get("rows", [])
    expected = {
        (budget, repetition, position_id, arm)
        for budget in ("fixed_nodes", "fixed_time")
        for repetition in range(3)
        for position_id in ids
        for arm in ("material", "cost-only", "teacher")
    }
    actual = {
        (row.get("budget"), row.get("repetition"), row.get("position_id"), row.get("arm"))
        for row in rows
    }
    if actual != expected or len(rows) != len(expected):
        errors.append("search run matrix is incomplete or duplicated")
    for row in rows:
        result = row.get("result", {})
        if not result.get("completed_iteration_valid"):
            errors.append(f"invalid completed iteration: {row.get('position_id')}")
        if not all(
            result.get(key)
            for key in ("pv_legal", "pv_replay_preserves_input", "history_matches_expected")
        ):
            errors.append(f"state/PV validation failed: {row.get('position_id')}")
        if not isinstance(result.get("max_rss_bytes"), int) or result["max_rss_bytes"] <= 0:
            errors.append(f"missing max RSS: {row.get('position_id')}")
    components = profile.get("component_runs", [])
    component_matrix = {
        (item.get("repetition"), item.get("mode")) for item in components
    }
    expected_components = {
        (repetition, mode)
        for repetition in range(3)
        for mode in ("material", "nnue-update", "forward")
    }
    if component_matrix != expected_components or len(components) != 9:
        errors.append("component run matrix is incomplete or duplicated")
    identity = summary.get("zero_scale_fixed_node_identity", {})
    if identity.get("comparisons") != 27:
        errors.append("zero-scale identity comparison count is not 27")
    attribution = summary.get("forward_attribution", {})
    if len(attribution.get("positions", [])) != 9:
        errors.append("forward attribution does not cover nine positions")
    result = {
        "valid": not errors,
        "errors": errors,
        "positions": len(ids),
        "search_runs": len(rows),
        "component_runs": len(components),
        "zero_scale_identity": identity,
    }
    print(json.dumps(result, sort_keys=True))
    return 0 if not errors else 1


if __name__ == "__main__":
    raise SystemExit(main())
