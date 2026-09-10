#!/usr/bin/env python3
"""Validate the provenance and compatibility contract for an external eval.

This is intentionally a stdlib-only validator.  It does not claim that an
external file is numerically compatible merely because its manifest parses;
the engine adapter must still pass its binary and inference probes.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
from typing import Any


SCHEMA = "external_eval_manifest_v1"
FORMAT_FAMILIES = {"yaneuraou_sfnn", "sekirei_nnue"}
REDISTRIBUTION_POLICIES = {"allowed", "restricted", "unknown", "not_redistributable"}


class ManifestError(ValueError):
    pass


def require(mapping: dict[str, Any], key: str, context: str) -> Any:
    if key not in mapping:
        raise ManifestError(f"{context}: missing {key}")
    return mapping[key]


def string(mapping: dict[str, Any], key: str, context: str, *, allow_empty=False) -> str:
    value = require(mapping, key, context)
    if not isinstance(value, str) or (not allow_empty and not value.strip()):
        raise ManifestError(f"{context}.{key}: expected non-empty string")
    return value


def sha256(value: Any, context: str) -> None:
    if not isinstance(value, str) or len(value) != 64:
        raise ManifestError(f"{context}: expected 64-character SHA-256")
    try:
        int(value, 16)
    except ValueError as error:
        raise ManifestError(f"{context}: expected hexadecimal SHA-256") from error


def validate(manifest: Any, *, check_artifacts=False, base_dir: Path | None = None) -> None:
    if not isinstance(manifest, dict):
        raise ManifestError("manifest: expected JSON object")
    if require(manifest, "schema", "manifest") != SCHEMA:
        raise ManifestError(f"manifest.schema: expected {SCHEMA}")

    artifact = require(manifest, "artifact", "manifest")
    if not isinstance(artifact, dict):
        raise ManifestError("manifest.artifact: expected object")
    artifact_path = string(artifact, "path", "manifest.artifact")
    size = require(artifact, "bytes", "manifest.artifact")
    if not isinstance(size, int) or size <= 0:
        raise ManifestError("manifest.artifact.bytes: expected positive integer")
    sha256(require(artifact, "sha256", "manifest.artifact"), "manifest.artifact.sha256")

    fmt = require(manifest, "format", "manifest")
    if not isinstance(fmt, dict):
        raise ManifestError("manifest.format: expected object")
    family = string(fmt, "family", "manifest.format")
    if family not in FORMAT_FAMILIES:
        raise ManifestError(f"manifest.format.family: unsupported family {family!r}")
    string(fmt, "version", "manifest.format")
    string(fmt, "architecture", "manifest.format")
    stacks = require(fmt, "layer_stacks", "manifest.format")
    if not isinstance(stacks, int) or not 1 <= stacks <= 4096:
        raise ManifestError("manifest.format.layer_stacks: expected 1..4096")
    endian = string(fmt, "endianness", "manifest.format")
    if endian != "little":
        raise ManifestError("manifest.format.endianness: only little is accepted")

    progress = require(manifest, "progress", "manifest")
    if not isinstance(progress, dict):
        raise ManifestError("manifest.progress: expected object")
    progress_path = require(progress, "path", "manifest.progress")
    if progress_path is not None and not isinstance(progress_path, str):
        raise ManifestError("manifest.progress.path: expected string or null")
    progress_sha = require(progress, "sha256", "manifest.progress")
    if progress_sha is not None:
        sha256(progress_sha, "manifest.progress.sha256")

    provenance = require(manifest, "provenance", "manifest")
    if not isinstance(provenance, dict):
        raise ManifestError("manifest.provenance: expected object")
    string(provenance, "source_url", "manifest.provenance")
    string(provenance, "source_commit", "manifest.provenance")
    string(provenance, "license", "manifest.provenance")
    policy = string(provenance, "redistribution", "manifest.provenance")
    if policy not in REDISTRIBUTION_POLICIES:
        raise ManifestError(f"manifest.provenance.redistribution: unsupported {policy!r}")

    compatibility = require(manifest, "compatibility", "manifest")
    if not isinstance(compatibility, dict):
        raise ManifestError("manifest.compatibility: expected object")
    string(compatibility, "adapter", "manifest.compatibility")
    status = string(compatibility, "status", "manifest.compatibility")
    if status not in {"declared", "parser_passed", "inference_passed", "rejected"}:
        raise ManifestError(f"manifest.compatibility.status: unsupported {status!r}")
    if status == "rejected":
        string(compatibility, "rejection_reason", "manifest.compatibility")

    if check_artifacts:
        root = base_dir or Path.cwd()
        for relative, expected, label in (
            (artifact_path, artifact["sha256"], "artifact"),
            (progress_path, progress_sha, "progress"),
        ):
            if relative is None:
                continue
            path = (root / relative).resolve()
            if not path.is_file():
                raise ManifestError(f"manifest.{label}.path: file not found: {relative}")
            actual = hashlib.sha256(path.read_bytes()).hexdigest()
            if actual != expected:
                raise ManifestError(f"manifest.{label}.sha256: mismatch for {relative}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--check-artifacts", action="store_true")
    args = parser.parse_args()
    try:
        document = json.loads(args.manifest.read_text(encoding="utf-8"))
        validate(document, check_artifacts=args.check_artifacts, base_dir=args.manifest.parent)
    except (OSError, json.JSONDecodeError, ManifestError) as error:
        print(f"invalid: {error}")
        return 1
    print(f"valid: {args.manifest} ({SCHEMA})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
