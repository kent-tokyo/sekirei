#!/usr/bin/env python3
"""Finalize a CSA startup manifest with immutable binary/weights hashes."""
from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
from pathlib import Path


SCHEMA = "sekirei.csa-run-manifest.v1"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def artifact(root: Path, value: object) -> dict:
    if not isinstance(value, str) or not value:
        return {"status": "unknown"}
    path = Path(value).expanduser().resolve()
    if not path.is_file():
        return {"status": "missing", "path": str(path)}
    result = {
        "status": "verified",
        "path": str(path),
        "bytes": path.stat().st_size,
        "sha256": sha256(path),
    }
    try:
        result["relative_path"] = path.relative_to(root).as_posix()
    except ValueError:
        result["relative_path"] = None
    return result


def artifact_input(value: object) -> tuple[object, object | None]:
    """Read both legacy path values and startup-manifest artifact objects."""
    if isinstance(value, dict):
        return value.get("path"), value.get("active")
    return value, None


def source_revision(document: dict, root: Path) -> str:
    current = document.get("source_revision")
    if isinstance(current, str) and current not in {"", "unknown"}:
        return current
    try:
        result = subprocess.run(
            ["git", "-C", str(root), "rev-parse", "HEAD"],
            check=True,
            capture_output=True,
            text=True,
        )
    except (OSError, subprocess.CalledProcessError):
        return current if isinstance(current, str) and current else "unknown"
    revision = result.stdout.strip()
    return revision or "unknown"


def finalize(source: Path, output: Path, root: Path) -> dict:
    document = json.loads(source.read_text(encoding="utf-8"))
    if document.get("schema") != SCHEMA:
        raise ValueError("unexpected CSA run manifest schema")
    binary = document.get("binary", {})
    weights = document.get("weights")
    binary_path, _ = artifact_input(binary)
    weights_path, weights_active = artifact_input(weights)
    document["binary"] = artifact(root, binary_path)
    document["weights"] = artifact(root, weights_path)
    if weights_active is not None:
        document["weights"]["active"] = weights_active
    document["source_revision"] = source_revision(document, root)
    document["status"] = "finalized" if document["binary"]["status"] == "verified" else "incomplete"
    document["hashes"] = {
        "binary": document["binary"].get("sha256"),
        "weights": document["weights"].get("sha256"),
    }
    output.write_text(json.dumps(document, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return document


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--root", type=Path, default=Path.cwd())
    args = parser.parse_args()
    document = finalize(args.source, args.output, args.root.resolve())
    print(f"CSA run manifest finalized: {args.output} ({document['status']})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
