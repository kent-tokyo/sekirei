#!/usr/bin/env python3
"""Validate version and build provenance in a generated WASM package or archive."""

from __future__ import annotations

import argparse
import json
import re
import tarfile
from pathlib import Path


VERSION_TOKEN = "X.Y.Z"
VERSION_RE = re.compile(r"\bv(\d+\.\d+\.\d+)\b")
ARCHIVE_RE = re.compile(r"sekirei-wasm-(\d+\.\d+\.\d+)\.tgz")
HEX40 = re.compile(r"[0-9a-f]{40}\Z")


def render_readme(template: str, version: str) -> str:
    if VERSION_TOKEN not in template:
        raise ValueError(f"WASM README is missing {VERSION_TOKEN}")
    rendered = template.replace(VERSION_TOKEN, version)
    if VERSION_TOKEN in rendered:
        raise ValueError("WASM README still contains an unresolved version token")
    return rendered


def validate_package(
    package_json_bytes: bytes,
    readme: str,
    asset_name: str,
    release_manifest: dict | None = None,
) -> list[str]:
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
    build = metadata.get("sekireiBuild")
    if not isinstance(build, dict):
        # Archives published through v0.5.0 predate the provenance contract.
        # Keep them inspectable; new packages must carry the contract.
        if tuple(map(int, version.split("."))) >= (0, 5, 1):
            errors.append("package.json sekireiBuild is missing")
        if release_manifest is not None and (
            not isinstance(release_manifest, dict)
            or release_manifest.get("release") != f"v{version}"
        ):
            errors.append("release manifest version differs from package version")
        return errors
    if build.get("apiVersion") != 4:
        errors.append("sekireiBuild.apiVersion must be 4")
    if build.get("engineVersion") != version:
        errors.append("sekireiBuild.engineVersion differs from package version")
    commit = build.get("sourceCommit")
    build_commit = build.get("buildCommit")
    status = build.get("sourceCommitStatus")
    provenance = build.get("provenance")
    known = (isinstance(commit, str) and HEX40.fullmatch(commit) is not None
             and isinstance(build_commit, str) and HEX40.fullmatch(build_commit) is not None)
    if not ((known and status == "known" and provenance == "release")
            or (known and status == "known" and provenance == "candidate"
                and commit == build_commit)
            or (commit is None and build_commit is None
                and status == "unknown" and provenance == "local")):
        errors.append("sekireiBuild source/build commit, status, or provenance are inconsistent")
    if (build.get("initialEvaluatorId") != "material"
            or build.get("initialEvaluatorVersion") != "material-v1"
            or build.get("initialEvaluatorWeightSha256", "missing") is not None):
        errors.append("sekireiBuild initial evaluator must be material-v1 without weights")
    if release_manifest is not None:
        if not isinstance(release_manifest, dict):
            errors.append("release manifest must be an object")
        elif (release_manifest.get("release") != f"v{version}"
              or release_manifest.get("commit") != commit
              or provenance != "release"):
            errors.append("sekireiBuild version/source commit does not match release manifest")
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
    parser.add_argument("--release-manifest", type=Path)
    args = parser.parse_args()

    try:
        release_manifest = (
            json.loads(args.release_manifest.read_text(encoding="utf-8"))
            if args.release_manifest else None
        )
        if args.package_dir:
            package_json = (args.package_dir / "package.json").read_bytes()
            readme = (args.package_dir / "README.md").read_text(encoding="utf-8")
            asset_name = args.asset_name or ""
        else:
            package_json, readme = read_archive(args.archive)
            asset_name = args.asset_name or args.archive.name
    except (OSError, UnicodeDecodeError, ValueError, tarfile.TarError, json.JSONDecodeError) as error:
        print(f"error: {error}")
        return 1

    errors = validate_package(package_json, readme, asset_name, release_manifest)
    for error in errors:
        print(f"error: {error}")
    if errors:
        return 1
    print("WASM package metadata OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
