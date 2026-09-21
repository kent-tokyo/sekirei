#!/usr/bin/env python3
"""Validate hashes, coverage, caps, and leakage for a Q21h frozen split."""

from __future__ import annotations

import argparse
import importlib.util
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "q21h", ROOT / "scripts" / "freeze_q21h_independent_split.py"
)
assert SPEC and SPEC.loader
Q21H = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(Q21H)


def read_jsonl(path: Path) -> list[dict]:
    return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]


def validate(manifest_path: Path) -> dict:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    errors: list[str] = []
    if manifest.get("schema") != "sekirei.q21h-independent-learning-split.v1":
        errors.append("unexpected schema")
    rows: dict[str, list[dict]] = {}
    identities: dict[str, set[str]] = {}
    groups: dict[str, set[str]] = {}
    for split in ("train", "validation"):
        path = Path(manifest[split]["path"])
        if Q21H.sha256(path) != manifest[split]["sha256"]:
            errors.append(f"{split} sha256 mismatch")
        rows[split] = read_jsonl(path)
        identities[split] = {Q21H.symmetry_key(row["sfen"]) for row in rows[split]}
        groups[split] = {row["source"]["derived_group"] for row in rows[split]}
        summary = Q21H.summarize(rows[split])
        for key in ("positions", "sources", "derived_groups", "phase_material_positions", "phase_material_sources", "maximum_positions_per_source"):
            if summary[key] != manifest[split][key]:
                errors.append(f"{split} summary mismatch: {key}")
        if summary["maximum_positions_per_source"] > manifest["contract"]["source_cap"]:
            errors.append(f"{split} source cap exceeded")
        expected_coverage = Q21H.coverage(summary, train=split == "train")
        if expected_coverage != manifest[split]["coverage"]:
            errors.append(f"{split} coverage mismatch")
    if identities["train"] & identities["validation"]:
        errors.append("train/validation symmetry identity overlap")
    if groups["train"] & groups["validation"]:
        errors.append("train/validation derived group overlap")

    exclusion_roots = [Path(path) for path in manifest["inputs"]["exclusion_roots"]]
    excluded, records = Q21H.load_exclusions(exclusion_roots)
    if len(excluded) != manifest["audit"]["excluded_symmetry_identities"]:
        errors.append("exclusion identity count mismatch")
    for split in ("train", "validation"):
        if identities[split] & excluded:
            errors.append(f"{split}/exclusion overlap")
    exclusions_path = manifest_path.parent / "exclusions.json"
    if json.loads(exclusions_path.read_text(encoding="utf-8")) != records:
        errors.append("exclusion ledger mismatch")

    ready = all(manifest[split]["coverage"]["passed"] for split in ("train", "validation")) and not errors
    if ready != manifest.get("selection_ready"):
        errors.append("selection_ready mismatch")
    return {
        "schema": "sekirei.q21h-independent-learning-split-validation.v1",
        "valid": not errors,
        "selection_ready": ready and not errors,
        "errors": errors,
        "train_positions": len(rows.get("train", [])),
        "validation_positions": len(rows.get("validation", [])),
        "train_validation_symmetry_overlap": len(identities.get("train", set()) & identities.get("validation", set())),
        "train_validation_derived_group_overlap": len(groups.get("train", set()) & groups.get("validation", set())),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    result = validate(args.manifest)
    text = json.dumps(result, ensure_ascii=False, indent=2) + "\n"
    if args.output:
        args.output.write_text(text, encoding="utf-8")
    print(text, end="")
    return 0 if result["valid"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
