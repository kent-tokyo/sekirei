#!/usr/bin/env python3
"""Validate shogiesa observations used only for Quietset diagnostics.

This deliberately implements the diagnostic-weighting mode from issue #95:
shogiesa observations may determine stability weights, but Sekirei's internal
search remains the only teacher-label source.  The emitted contract is small
enough to hash and attach to the trainer metadata.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path
from typing import Any

SUPPORTED_SHOGIESA_VERSIONS = {"0.11.2"}
SUPPORTED_SCHEMA_VERSION = 11
TEACHER_SOURCE = "sekirei_internal_search"


class ContractError(ValueError):
    pass


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def parse_depths(value: str) -> list[int]:
    try:
        depths = [int(part) for part in value.split(",") if part]
    except ValueError as error:
        raise ContractError(f"invalid depth list {value!r}") from error
    if not depths or any(depth <= 0 for depth in depths) or len(depths) != len(set(depths)):
        raise ContractError("depths must be unique positive integers")
    return depths


def parse_version_output(output: str) -> str:
    match = re.search(r"\b(\d+\.\d+\.\d+)\b", output.strip())
    if not match:
        raise ContractError(f"cannot parse shogiesa version from {output!r}")
    version = match.group(1)
    if version not in SUPPORTED_SHOGIESA_VERSIONS:
        supported = ", ".join(sorted(SUPPORTED_SHOGIESA_VERSIONS))
        raise ContractError(f"unsupported shogiesa version {version}; supported: {supported}")
    return version


def read_version(executable: str) -> str:
    completed = subprocess.run(
        [executable, "--version"],
        check=False,
        capture_output=True,
        text=True,
    )
    if completed.returncode != 0:
        raise ContractError(
            f"{executable} --version failed with exit code {completed.returncode}"
        )
    return parse_version_output(completed.stdout or completed.stderr)


def load_object(path: Path, description: str) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ContractError(f"cannot read {description} {path}: {error}") from error
    if not isinstance(value, dict):
        raise ContractError(f"{description} must be a JSON object")
    return value


def validate_manifest(
    manifest: dict[str, Any], version: str, expected_depths: list[int]
) -> dict[str, int | float | None]:
    if manifest.get("shogiesa_version") != version:
        raise ContractError("manifest shogiesa_version does not match the executable")
    if manifest.get("schema_version") != SUPPORTED_SCHEMA_VERSION:
        raise ContractError(
            f"unsupported manifest schema_version {manifest.get('schema_version')!r}"
        )
    if manifest.get("command") != "label":
        raise ContractError("shogiesa manifest command must be 'label'")
    if manifest.get("depths") != expected_depths:
        raise ContractError("manifest depths do not match the requested diagnostic depths")
    if manifest.get("requested_depth_underreach") != 0:
        raise ContractError("manifest records under-reached non-mate depth requests")
    if manifest.get("requested_depth_total", 0) <= 0:
        raise ContractError("manifest has no requested-depth provenance")
    options = manifest.get("engine_options")
    if not isinstance(options, list) or "UseBook=false" not in options:
        raise ContractError("diagnostic label run must record engine option UseBook=false")
    return {
        "records_read": manifest.get("records_read"),
        "records_kept": manifest.get("records_kept"),
        "cache_hits": manifest.get("cache_hits"),
        "cache_misses": manifest.get("cache_misses"),
        "cache_hit_rate": manifest.get("cache_hit_rate"),
        "requested_depth_total": manifest.get("requested_depth_total"),
        "requested_depth_underreach": manifest.get("requested_depth_underreach"),
    }


def validate_observations(path: Path, expected_depths: list[int]) -> dict[str, int]:
    records = 0
    observations = 0
    mate_observations = 0
    requested_depth_total = 0
    expected = set(expected_depths)
    try:
        lines = path.open(encoding="utf-8")
    except OSError as error:
        raise ContractError(f"cannot read observations {path}: {error}") from error
    with lines:
        for line_number, raw in enumerate(lines, 1):
            if not raw.strip():
                continue
            try:
                record = json.loads(raw)
            except json.JSONDecodeError as error:
                raise ContractError(
                    f"observations line {line_number}: invalid JSON: {error}"
                ) from error
            if not isinstance(record, dict):
                raise ContractError(f"observations line {line_number}: expected object")
            if record.get("schema_version") != SUPPORTED_SCHEMA_VERSION:
                raise ContractError(
                    f"observations line {line_number}: unsupported schema_version"
                )
            row_observations = record.get("observations")
            if not isinstance(row_observations, list) or not row_observations:
                raise ContractError(
                    f"observations line {line_number}: missing observation provenance"
                )
            seen_depths: set[int] = set()
            for observation in row_observations:
                if not isinstance(observation, dict):
                    raise ContractError(
                        f"observations line {line_number}: observation must be an object"
                    )
                requested = observation.get("requested_depth")
                achieved = observation.get("depth")
                if requested not in expected or not isinstance(achieved, int):
                    raise ContractError(
                        f"observations line {line_number}: missing or unexpected requested/achieved depth"
                    )
                if requested in seen_depths:
                    raise ContractError(
                        f"observations line {line_number}: duplicate requested depth {requested}"
                    )
                seen_depths.add(requested)
                if observation.get("search_limit_kind") != "depth":
                    raise ContractError(
                        f"observations line {line_number}: expected depth-limited provenance"
                    )
                if observation.get("score_perspective") not in {
                    "side_to_move",
                    "black",
                    "white",
                }:
                    raise ContractError(
                        f"observations line {line_number}: missing score perspective"
                    )
                if observation.get("score_bound") not in {
                    "exact",
                    "lowerbound",
                    "upperbound",
                }:
                    raise ContractError(
                        f"observations line {line_number}: missing score bound"
                    )
                score = observation.get("score")
                score_kind = score.get("kind") if isinstance(score, dict) else None
                if score_kind not in {"cp", "mate"}:
                    raise ContractError(
                        f"observations line {line_number}: unsupported score provenance"
                    )
                if achieved < requested and score_kind != "mate":
                    raise ContractError(
                        f"observations line {line_number}: depth {requested} under-reached at {achieved}"
                    )
                requested_depth_total += 1
                observations += 1
                mate_observations += int(score_kind == "mate")
            if seen_depths != expected:
                raise ContractError(
                    f"observations line {line_number}: expected depths {sorted(expected)}, got {sorted(seen_depths)}"
                )
            records += 1
    if records == 0:
        raise ContractError("observations JSONL is empty")
    return {
        "records": records,
        "observations": observations,
        "mate_observations": mate_observations,
        "requested_depth_total": requested_depth_total,
    }


def build_contract(
    *,
    version: str,
    manifest_path: Path,
    observations_path: Path,
    expected_depths: list[int],
    label_depth: int,
    depth_mismatch_reason: str | None,
    label_elapsed_seconds: int | None,
) -> dict[str, Any]:
    if label_depth != max(expected_depths) and not depth_mismatch_reason:
        raise ContractError(
            "internal label depth differs from the deepest diagnostic depth; "
            "supply --depth-mismatch-reason to record an intentional mismatch"
        )
    manifest = load_object(manifest_path, "shogiesa manifest")
    manifest_stats = validate_manifest(manifest, version, expected_depths)
    observation_stats = validate_observations(observations_path, expected_depths)
    if manifest_stats["requested_depth_total"] != observation_stats["requested_depth_total"]:
        raise ContractError("manifest and observations disagree on requested-depth coverage")
    return {
        "contract_schema_version": 1,
        "mode": "diagnostic-weighting",
        "teacher_source": TEACHER_SOURCE,
        "diagnostic_observations_used_as_teacher": False,
        "score_perspective_policy": "validated_but_not_converted_diagnostic_only",
        "target_equality_contract": "shogiesa observations are excluded from teacher targets",
        "shogiesa_version": version,
        "shogiesa_schema_version": SUPPORTED_SCHEMA_VERSION,
        "requested_depths": expected_depths,
        "sekirei_internal_label_depth": label_depth,
        "depth_mismatch_reason": depth_mismatch_reason,
        "shogiesa_manifest": str(manifest_path),
        "shogiesa_manifest_sha256": sha256_file(manifest_path),
        "observations": str(observations_path),
        "observations_sha256": sha256_file(observations_path),
        "shogiesa_label_elapsed_seconds": label_elapsed_seconds,
        "shogiesa_manifest_stats": manifest_stats,
        "observation_stats": observation_stats,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--shogiesa", required=True)
    parser.add_argument("--version-only", action="store_true")
    parser.add_argument("--manifest", type=Path)
    parser.add_argument("--observations", type=Path)
    parser.add_argument("--depths")
    parser.add_argument("--label-depth", type=int)
    parser.add_argument("--depth-mismatch-reason")
    parser.add_argument("--label-elapsed-seconds", type=int)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    try:
        version = read_version(args.shogiesa)
        if args.version_only:
            print(version)
            return 0
        required = {
            "--manifest": args.manifest,
            "--observations": args.observations,
            "--depths": args.depths,
            "--label-depth": args.label_depth,
            "--output": args.output,
        }
        missing = [name for name, value in required.items() if value is None]
        if missing:
            raise ContractError(f"missing required arguments: {', '.join(missing)}")
        contract = build_contract(
            version=version,
            manifest_path=args.manifest,
            observations_path=args.observations,
            expected_depths=parse_depths(args.depths),
            label_depth=args.label_depth,
            depth_mismatch_reason=args.depth_mismatch_reason,
            label_elapsed_seconds=args.label_elapsed_seconds,
        )
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(
            json.dumps(contract, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
        # stdout is deliberately machine-readable for shell wrappers.
        print(contract["shogiesa_manifest_sha256"])
        return 0
    except (ContractError, OSError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
