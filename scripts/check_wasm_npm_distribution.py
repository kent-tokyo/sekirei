#!/usr/bin/env python3
"""Validate the exact files distributed by GitHub Release and npm."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import tarfile
import tomllib
from pathlib import Path, PurePosixPath

from check_wasm_package_metadata import validate_package


REQUIRED = {
    "package.json", "README.md", "NOTICE", "LICENSE-MIT", "LICENSE-APACHE",
    "sekirei_wasm.js", "sekirei_wasm.d.ts", "sekirei_wasm_bg.wasm",
}
VERSION = re.compile(r"^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$")


def archive_files(path: Path) -> dict[str, bytes]:
    """Read a package tarball without accepting links, duplicates, or traversal."""
    files: dict[str, bytes] = {}
    with tarfile.open(path, "r:gz") as archive:
        for member in archive:
            if member.isdir():
                continue
            parts = PurePosixPath(member.name).parts
            if (len(parts) < 2 or parts[0] != "package" or
                    any(part in {".", ".."} for part in parts) or not member.isfile()):
                raise ValueError(f"unexpected tar entry: {member.name}")
            name = "/".join(parts[1:])
            if name in files:
                raise ValueError(f"duplicate tar entry: {name}")
            stream = archive.extractfile(member)
            if stream is None:
                raise ValueError(f"unreadable tar entry: {name}")
            files[name] = stream.read()
    return files


def check_archive(files: dict[str, bytes], version: str) -> list[str]:
    errors = [f"missing package file: {name}" for name in sorted(REQUIRED - files.keys())]
    errors.extend(
        f"forbidden package file: {name}" for name in sorted(files)
        if ("nn_r6" in name.lower() or "weight" in name.lower() or
            name.lower().endswith((".bin", ".nnue", ".xz", ".onnx", ".safetensors", ".pt")))
    )
    if "package.json" not in files or "README.md" not in files:
        return errors
    try:
        metadata = json.loads(files["package.json"])
        readme = files["README.md"].decode("utf-8")
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        return errors + [f"invalid package metadata: {exc}"]
    if metadata.get("name") != "sekirei-wasm":
        errors.append("package name must be sekirei-wasm")
    if metadata.get("version") != version:
        errors.append("package version does not match release tag")
    errors.extend(validate_package(files["package.json"], readme, f"sekirei-wasm-{version}.tgz"))
    if tuple(map(int, version.split("."))) >= (0, 5, 1):
        if f"npm install sekirei-wasm@{version}" not in readme:
            errors.append("README is missing the version-pinned npm registry install example")
    declarations = files.get("sekirei_wasm.d.ts", b"")
    for symbol in (b"startSfen", b"searchCapabilities", b"analyzePosition"):
        if symbol not in declarations:
            errors.append(f"generated declarations lack {symbol.decode()}")
    return errors


def compare_files(release: dict[str, bytes], npm: dict[str, bytes]) -> list[str]:
    errors = []
    for name in sorted(release.keys() | npm.keys()):
        if name not in release or name not in npm:
            errors.append(f"file list differs: {name}")
        elif hashlib.sha256(release[name]).digest() != hashlib.sha256(npm[name]).digest():
            errors.append(f"file SHA-256 differs: {name}")
    return errors


def validate_release_tag(tag: str, cargo_manifest: Path, release_manifest: Path,
                         archive: Path) -> tuple[str, list[str]]:
    match = VERSION.fullmatch(tag)
    if not match:
        return "", ["release tag must be a stable vMAJOR.MINOR.PATCH tag"]
    version = tag[1:]
    errors = []
    cargo_version = tomllib.loads(cargo_manifest.read_text(encoding="utf-8"))["package"]["version"]
    if cargo_version != version:
        errors.append(f"Cargo version {cargo_version} != {version}")
    manifest = json.loads(release_manifest.read_text(encoding="utf-8"))
    wasm = manifest.get("webassembly", {})
    if manifest.get("release") != tag or wasm.get("version") != version:
        errors.append("release manifest version does not match tag")
    if wasm.get("asset") != archive.name:
        errors.append("release manifest asset name does not match archive")
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    if wasm.get("sha256") != digest or wasm.get("size") != archive.stat().st_size:
        errors.append("release archive SHA-256 or size differs from manifest")
    return version, errors


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release-archive", required=True, type=Path)
    parser.add_argument("--npm-archive", type=Path)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--cargo-manifest", type=Path, default=Path("crates/sekirei-wasm/Cargo.toml"))
    parser.add_argument("--release-manifest", required=True, type=Path)
    args = parser.parse_args()
    try:
        version, errors = validate_release_tag(args.tag, args.cargo_manifest,
                                               args.release_manifest, args.release_archive)
        if version:
            release = archive_files(args.release_archive)
            errors.extend(check_archive(release, version))
            if args.npm_archive:
                npm = archive_files(args.npm_archive)
                errors.extend(check_archive(npm, version))
                errors.extend(compare_files(release, npm))
    except (OSError, ValueError, KeyError, tarfile.TarError) as exc:
        errors = [str(exc)]
    for error in errors:
        print(f"error: {error}")
    if errors:
        return 1
    print(f"WASM release/npm package files match v{version}" if args.npm_archive
          else f"WASM release package is valid v{version}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
