#!/usr/bin/env python3
"""Attach hashed, diagnostic-only artifacts to a release manifest copy."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

from validate_release_manifest import validate as validate_release


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def attach(release_path: Path, artifacts: list[Path], output_path: Path) -> dict:
    release = json.loads(release_path.read_text(encoding="utf-8"))
    errors = validate_release(release)
    if errors:
        raise ValueError("invalid release manifest: " + ", ".join(errors))
    entries = []
    for path in artifacts:
        document = json.loads(path.read_text(encoding="utf-8"))
        if document.get("diagnostic_only") is not True:
            raise ValueError(f"artifact is not diagnostic-only: {path}")
        schema = document.get("schema")
        if not isinstance(schema, str) or not schema:
            raise ValueError(f"artifact schema is missing: {path}")
        entries.append({"path": str(path), "sha256": sha256(path), "schema": schema})
    result = dict(release)
    result["diagnostic_artifacts"] = {
        "diagnostic_only": True,
        "strength_claim": "not_permitted",
        "artifacts": entries,
    }
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return result


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release-manifest", type=Path, required=True)
    parser.add_argument("--artifact", type=Path, action="append", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    attach(args.release_manifest, args.artifact, args.output)
    print(f"diagnostic artifact manifest copy written: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
