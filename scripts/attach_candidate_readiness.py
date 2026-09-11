#!/usr/bin/env python3
"""Attach a verified candidate-readiness report to a manifest copy."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from validate_release_manifest import validate as validate_release


def attach(release_path: Path, readiness_path: Path, output_path: Path) -> dict:
    release = json.loads(release_path.read_text(encoding="utf-8"))
    readiness = json.loads(readiness_path.read_text(encoding="utf-8"))
    errors = validate_release(release)
    if errors:
        raise ValueError("invalid release manifest: " + ", ".join(errors))
    if readiness.get("schema") != "sekirei.candidate-readiness.v1":
        raise ValueError("invalid candidate-readiness schema")
    if not isinstance(readiness.get("strict_probe"), dict):
        raise ValueError("candidate-readiness strict_probe must be an object")
    result = dict(release)
    result["candidate_readiness"] = readiness
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return result


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release-manifest", type=Path, required=True)
    parser.add_argument("--readiness", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    attach(args.release_manifest, args.readiness, args.output)
    print(f"candidate readiness manifest copy written: {args.output}")


if __name__ == "__main__":
    main()
