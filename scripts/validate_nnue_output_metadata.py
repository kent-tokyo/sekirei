#!/usr/bin/env python3
"""Fail closed on NNUE output-mode metadata before a candidate is measured.

`SEKIRW01` intentionally remains a stable binary layout, so whether its value
is absolute or a residual must be bound by a sidecar that also fingerprints the
exact bytes.  This tool is deliberately dependency-free for use by local gates.
"""

from __future__ import annotations

import argparse
import json
from pathlib import Path


FNV_OFFSET = 14695981039346656037
FNV_PRIME = 1099511628211
MASK64 = (1 << 64) - 1


def checkpoint_hash(path: Path) -> str:
    value = FNV_OFFSET
    for byte in path.read_bytes():
        value = ((value ^ byte) * FNV_PRIME) & MASK64
    return f"{value:016x}"


def metadata_path(weights: Path) -> Path:
    return weights.with_suffix(".meta.json")


def validate(weights: Path, expected_mode: str | None = None) -> dict:
    if not weights.is_file():
        raise ValueError(f"weights file does not exist: {weights}")
    sidecar = metadata_path(weights)
    if not sidecar.is_file():
        raise ValueError(f"missing NNUE output metadata: {sidecar}")
    try:
        metadata = json.loads(sidecar.read_text(encoding="utf-8"))
    except json.JSONDecodeError as error:
        raise ValueError(f"invalid NNUE output metadata {sidecar}: {error}") from error
    if metadata.get("format") != "sekirei-nnue-output-v1":
        raise ValueError("unsupported NNUE output metadata format")
    mode = metadata.get("nnue_output")
    if mode not in {"absolute", "residual-material"}:
        raise ValueError(f"unsupported NNUE output mode: {mode!r}")
    if expected_mode is not None and mode != expected_mode:
        raise ValueError(f"NNUE output mode mismatch: expected {expected_mode}, got {mode}")
    if mode == "residual-material" and metadata.get("baseline") != "material-v1":
        raise ValueError("residual-material weights must declare baseline material-v1")
    if mode == "absolute" and metadata.get("baseline") is not None:
        raise ValueError("absolute weights must not declare a residual baseline")
    actual_hash = checkpoint_hash(weights)
    if metadata.get("checkpoint_hash") != actual_hash:
        raise ValueError("NNUE output metadata checkpoint_hash does not match weights")
    return {"weights": str(weights), "metadata": str(sidecar), "mode": mode, "hash": actual_hash}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("weights", type=Path)
    parser.add_argument("--expect-mode", choices=["absolute", "residual-material"])
    args = parser.parse_args()
    try:
        print(json.dumps(validate(args.weights, args.expect_mode), sort_keys=True))
    except ValueError as error:
        parser.error(str(error))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
