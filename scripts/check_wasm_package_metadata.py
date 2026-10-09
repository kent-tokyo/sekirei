#!/usr/bin/env python3
"""Validate version agreement in a generated Sekirei WASM package or archive."""

from __future__ import annotations

import argparse
import json
import re
import tarfile
from pathlib import Path


VERSION_TOKEN = "X.Y.Z"
VERSION_RE = re.compile(r"\bv(\d+\.\d+\.\d+)\b")
ARCHIVE_RE = re.compile(r"sekirei-wasm-(\d+\.\d+\.\d+)\.tgz")


def render_readme(template: str, version: str) -> str:
    if VERSION_TOKEN not in template:
        raise ValueError(f"WASM README is missing {VERSION_TOKEN}")
    rendered = template.replace(VERSION_TOKEN, version)
    if VERSION_TOKEN in rendered:
        raise ValueError("WASM README still contains an unresolved version token")
    return rendered


def validate_package(package_json_bytes: bytes, readme: str, asset_name: str) -> list[str]:
    errors: list[str] = []
    try:
        metadata = json.loads(package_json_bytes)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        return [f"package.json is invalid: {error}"]
    version = metadata.get("version")
    if not isinstance(version, str) or not re.fullmatch(r"\d+\.\d+\.\d+", version):
        return ["package.json version is missing or invalid"]

    if VERSION_TOKEN in readme:
        errors.append("README contains an unresolved version token")
    mentioned_versions = set(VERSION_RE.findall(readme)) | set(ARCHIVE_RE.findall(readme))
    if mentioned_versions != {version}:
        errors.append(
            f"README versions {sorted(mentioned_versions)} do not match package.json {version}"
        )
    expected_url = (
        f"https://github.com/kent-tokyo/sekirei/releases/download/v{version}/"
        f"sekirei-wasm-{version}.tgz"
    )
    if expected_url not in readme:
        errors.append(f"README is missing the current install URL: {expected_url}")
    expected_asset = f"sekirei-wasm-{version}.tgz"
    if Path(asset_name).name != expected_asset:
        errors.append(f"asset name {Path(asset_name).name} does not match {expected_asset}")
    return errors


def read_archive(path: Path) -> tuple[bytes, str]:
    with tarfile.open(path, "r:gz") as archive:
        try:
            package_json = archive.extractfile("package/package.json")
            readme = archive.extractfile("package/README.md")
        except KeyError as error:
            raise ValueError(
                "archive must contain package/package.json and package/README.md"
            ) from error
        if package_json is None or readme is None:
            raise ValueError("archive must contain package/package.json and package/README.md")
        return package_json.read(), readme.read().decode("utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--package-dir", type=Path)
    source.add_argument("--archive", type=Path)
    parser.add_argument("--asset-name")
    args = parser.parse_args()

    try:
        if args.package_dir:
            package_json = (args.package_dir / "package.json").read_bytes()
            readme = (args.package_dir / "README.md").read_text(encoding="utf-8")
            asset_name = args.asset_name or ""
        else:
            package_json, readme = read_archive(args.archive)
            asset_name = args.asset_name or args.archive.name
    except (OSError, UnicodeDecodeError, ValueError, tarfile.TarError) as error:
        print(f"error: {error}")
        return 1

    errors = validate_package(package_json, readme, asset_name)
    for error in errors:
        print(f"error: {error}")
    if errors:
        return 1
    print("WASM package metadata OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
