#!/usr/bin/env python3
"""Fail closed validation for a short, CPU-first self-NNUE compute pilot.

This tool validates only a frozen pilot plan.  It never provisions a cloud
instance, launches an engine, or trains a model.  A valid plan proves that a
future pilot has fixed inputs, measurable output, a bounded duration, and
explicit stop conditions; it does not authorize NNUE distribution or default
adoption.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path


SHA256 = re.compile(r"^[0-9a-f]{64}$")
GIT_COMMIT = re.compile(r"^[0-9a-f]{40}$")
SCHEMA = "sekirei.nnue-cpu-pilot.v1"
REQUIRED_METRICS = {
    "label_positions_per_second",
    "selfplay_positions_per_second",
    "training_positions_per_second",
    "peak_rss_bytes",
    "resume_verified",
    "artifact_sha256",
}
REQUIRED_STOP_CONDITIONS = {
    "max_elapsed_seconds",
    "out_of_memory",
    "nonzero_exit",
    "hash_mismatch",
    "resume_failure",
    "quality_regression",
}
REQUIRED_REFERENCES = {
    "candidate_card",
    "dataset_manifest",
    "whole_game_holdout_manifest",
    "checkpoint_resume_manifest",
    "material_gate_evidence",
    "local_cpu_measurement",
}


def digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def validate_reference(root: Path, value: object, field: str) -> list[str]:
    if not isinstance(value, dict):
        return [field]
    relative = value.get("path")
    expected = value.get("sha256")
    if not isinstance(relative, str) or not relative or not SHA256.fullmatch(expected or ""):
        return [field]
    if Path(relative).is_absolute():
        return [f"{field}.outside_root"]
    root = root.resolve()
    path = (root / relative).resolve()
    try:
        path.relative_to(root)
    except ValueError:
        return [f"{field}.outside_root"]
    if not path.is_file():
        return [f"{field}.missing"]
    return [] if digest(path) == expected else [f"{field}.sha256"]


def includes_required_names(value: object, required: set[str]) -> bool:
    """Return false, rather than raising, for malformed JSON arrays."""
    return isinstance(value, list) and all(isinstance(item, str) for item in value) and required <= set(value)


def validate(plan: object, root: Path) -> list[str]:
    if not isinstance(plan, dict):
        return ["document"]
    errors: list[str] = []
    if plan.get("schema") != SCHEMA:
        errors.append("schema")

    candidate = plan.get("candidate")
    if not isinstance(candidate, dict):
        errors.append("candidate")
    else:
        if not isinstance(candidate.get("id"), str) or not candidate["id"].strip():
            errors.append("candidate.id")
        if not SHA256.fullmatch(candidate.get("sha256", "")):
            errors.append("candidate.sha256")
        if not GIT_COMMIT.fullmatch(candidate.get("engine_commit", "")):
            errors.append("candidate.engine_commit")
        if candidate.get("evaluation") != "candidate-vs-material":
            errors.append("candidate.evaluation")
        if candidate.get("paired_colour_reversed") is not True:
            errors.append("candidate.paired_colour_reversed")
        if candidate.get("material_gate_verdict") != "PASS":
            errors.append("candidate.material_gate_verdict")

    evidence = plan.get("evidence")
    if not isinstance(evidence, dict):
        errors.append("evidence")
    else:
        for name in REQUIRED_REFERENCES:
            errors.extend(validate_reference(root, evidence.get(name), f"evidence.{name}"))

        bottleneck = evidence.get("bottleneck")
        if not isinstance(bottleneck, dict):
            errors.append("evidence.bottleneck")
        else:
            for name in ("code_correctness", "teacher_quality", "model_design"):
                if bottleneck.get(name) != "resolved":
                    errors.append(f"evidence.bottleneck.{name}")
            if bottleneck.get("cpu_dominant_phase") not in {
                "label_generation",
                "self_play",
                "training",
            }:
                errors.append("evidence.bottleneck.cpu_dominant_phase")
            if not isinstance(bottleneck.get("local_cpu_seconds"), (int, float)) or bottleneck["local_cpu_seconds"] <= 0:
                errors.append("evidence.bottleneck.local_cpu_seconds")
            if not isinstance(bottleneck.get("local_wall_seconds"), (int, float)) or bottleneck["local_wall_seconds"] <= 0:
                errors.append("evidence.bottleneck.local_wall_seconds")
            elif bottleneck.get("local_cpu_seconds", 0) < bottleneck["local_wall_seconds"] * 0.75:
                errors.append("evidence.bottleneck.cpu_not_dominant")

    pilot = plan.get("pilot")
    if not isinstance(pilot, dict):
        errors.append("pilot")
    else:
        if pilot.get("provider") != "aws-ec2":
            errors.append("pilot.provider")
        if not isinstance(pilot.get("instance_type"), str) or not pilot["instance_type"].strip():
            errors.append("pilot.instance_type")
        if not isinstance(pilot.get("vcpus"), int) or not 1 <= pilot["vcpus"] <= 8:
            errors.append("pilot.vcpus")
        if not isinstance(pilot.get("estimated_minutes"), int) or not 1 <= pilot["estimated_minutes"] <= 360:
            errors.append("pilot.estimated_minutes")
        if not isinstance(pilot.get("max_minutes"), int) or not 1 <= pilot["max_minutes"] <= 360:
            errors.append("pilot.max_minutes")
        elif isinstance(pilot.get("estimated_minutes"), int) and pilot["max_minutes"] < pilot["estimated_minutes"]:
            errors.append("pilot.max_minutes")
        if not isinstance(pilot.get("workload_positions"), int) or pilot["workload_positions"] <= 0:
            errors.append("pilot.workload_positions")
        metrics = pilot.get("metrics")
        if not includes_required_names(metrics, REQUIRED_METRICS):
            errors.append("pilot.metrics")
        stop_conditions = pilot.get("stop_conditions")
        if not includes_required_names(stop_conditions, REQUIRED_STOP_CONDITIONS):
            errors.append("pilot.stop_conditions")
    return errors


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("plan", type=Path)
    args = parser.parse_args(argv)
    try:
        plan = json.loads(args.plan.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        print(f"invalid CPU pilot plan: {error}", file=sys.stderr)
        return 2
    errors = validate(plan, args.plan.parent)
    if errors:
        print("invalid CPU pilot plan: " + ", ".join(errors), file=sys.stderr)
        return 1
    print(
        "valid CPU pilot plan: "
        f"candidate={plan['candidate']['id']} "
        f"workload_positions={plan['pilot']['workload_positions']} "
        f"max_minutes={plan['pilot']['max_minutes']}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
