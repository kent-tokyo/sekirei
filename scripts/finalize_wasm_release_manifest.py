#!/usr/bin/env python3
"""Record a tagged WASM asset digest after the immutable tag has been built."""

from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path

from check_wasm_npm_distribution import archive_files, check_archive
from validate_release_manifest import validate


def finalize(source: dict, asset: Path, checksum_text: str, tag_commit: str) -> dict:
    errors = validate(source, allow_planned_publish=True)
    if errors:
        raise ValueError("invalid planned release manifest: " + ", ".join(errors))
    wasm = source["webassembly"]
    if wasm.get("status") != "planned":
        raise ValueError("WASM release must be planned before finalization")
    if asset.name != wasm["asset"]:
        raise ValueError("WASM asset filename does not match release manifest")
    package_errors = check_archive(archive_files(asset), wasm["version"],
                                   source_commit=source.get("commit"),
                                   tag_commit=tag_commit)
    if package_errors:
        raise ValueError("invalid tagged WASM package: " + ", ".join(package_errors))
    expected = []
    for line in checksum_text.splitlines():
        fields = line.split()
        if len(fields) == 2 and fields[1] == asset.name:
            expected.append(fields[0])
    if len(expected) != 1:
        raise ValueError("SHA256SUMS must contain exactly one matching WASM asset entry")
    data = asset.read_bytes()
    digest = hashlib.sha256(data).hexdigest()
    if digest != expected[0]:
        raise ValueError("WASM asset digest differs from SHA256SUMS")
    output = json.loads(json.dumps(source))
    output["webassembly"].update(status="verified", sha256=digest, size=len(data))
    errors = validate(output, allow_planned_publish=True)
    if errors:
        raise ValueError("invalid finalized release manifest: " + ", ".join(errors))
    return output


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--planned-manifest", type=Path, required=True)
    parser.add_argument("--asset", type=Path, required=True)
    parser.add_argument("--sha256sums", type=Path, required=True)
    parser.add_argument("--tag-commit", required=True,
                        help="peeled commit of the immutable release tag")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        planned = json.loads(args.planned_manifest.read_text(encoding="utf-8"))
        finalized = finalize(planned, args.asset, args.sha256sums.read_text(encoding="utf-8"),
                             args.tag_commit)
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as exc:
        parser.error(str(exc))
    args.output.write_text(json.dumps(finalized, indent=2, ensure_ascii=False) + "\n",
                           encoding="utf-8")
    print(f"wrote verified WASM digest: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
