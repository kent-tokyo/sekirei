#!/usr/bin/env python3
"""Attach a finalized run-manifest hash to an analysis JSONL copy."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

from validate_csa_run_manifest import validate


def attach(manifest_path: Path, analysis_path: Path, output_path: Path) -> int:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    errors = validate(manifest, finalized=True)
    if errors:
        raise ValueError("invalid finalized run manifest: " + ", ".join(errors))
    manifest_hash = hashlib.sha256(manifest_path.read_bytes()).hexdigest()
    lines = analysis_path.read_text(encoding="utf-8").splitlines()
    if not lines:
        raise ValueError("analysis record is empty")
    try:
        header = json.loads(lines[0])
    except json.JSONDecodeError as exc:
        raise ValueError("analysis header is not JSON") from exc
    if header.get("schema") not in {"sekirei.analysis-record.v2", "sekirei.analysis-record.v3"}:
        raise ValueError("analysis header schema is unsupported")
    header["run_manifest_sha256"] = manifest_hash
    header["run_manifest_path"] = str(manifest_path)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(
        json.dumps(header, ensure_ascii=False, separators=(",", ":"))
        + "\n"
        + "\n".join(lines[1:])
        + "\n",
        encoding="utf-8",
    )
    return len(lines)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--analysis", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    count = attach(args.manifest, args.analysis, args.output)
    print(f"attached run manifest to {args.output}: {count} lines")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
