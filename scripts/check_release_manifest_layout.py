#!/usr/bin/env python3
"""Require one current root manifest and verified archived release evidence."""

from __future__ import annotations

import json
import re
import sys
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
ARCHIVE = ROOT / "docs/releases/manifests"
PATTERN = re.compile(r"release-manifest-v\d+\.\d+\.\d+\.json$")


def manifest_names(directory: Path) -> set[str]:
    if not directory.is_dir():
        return set()
    return {path.name for path in directory.iterdir() if path.is_file() and PATTERN.fullmatch(path.name)}


def check_layout(root: Path, current_version: str) -> list[str]:
    expected = f"release-manifest-v{current_version}.json"
    root_names = manifest_names(root)
    archive = root / "docs/releases/manifests"
    archive_names = manifest_names(archive)
    errors = []
    if root_names != {expected}:
        errors.append(
            "root release manifests must contain only "
            f"{expected}: found {sorted(root_names)}"
        )
    duplicate = sorted(root_names & archive_names)
    if duplicate:
        errors.append(f"release manifests appear in both root and archive: {duplicate}")
    for name in sorted(archive_names):
        try:
            document = json.loads((archive / name).read_text(encoding="utf-8"))
        except (OSError, ValueError) as error:
            errors.append(f"archived release manifest {name} is unreadable: {error}")
            continue
        if document.get("release") == f"v{current_version}":
            errors.append(f"current release manifest is archived: {name}")
        if document.get("publish", {}).get("status") != "verified":
            errors.append(f"archived release manifest is not verified: {name}")
        version = tuple(int(part) for part in name.removeprefix("release-manifest-v").removesuffix(".json").split("."))
        wasm = document.get("webassembly")
        if version >= (0, 5, 1) and (not isinstance(wasm, dict) or wasm.get("status") != "verified"):
            errors.append(f"archived WASM release manifest is not verified: {name}")
    return errors


def current_version(root: Path) -> str:
    manifest = root / "crates/sekirei-core/Cargo.toml"
    return tomllib.loads(manifest.read_text(encoding="utf-8"))["package"]["version"]


def main() -> int:
    errors = check_layout(ROOT, current_version(ROOT))
    if errors:
        for error in errors:
            print(f"error: {error}", file=sys.stderr)
        return 1
    print(f"release manifest layout OK: current={current_version(ROOT)} archived={len(manifest_names(ARCHIVE))}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
