#!/usr/bin/env python3
"""Build the browser package and add the repository's legal notices."""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
from pathlib import Path

from check_wasm_package_metadata import render_readme, validate_package


ROOT = Path(__file__).resolve().parents[1]
CRATE = ROOT / "crates" / "sekirei-wasm"
PACKAGE = CRATE / "pkg"
LEGAL_FILES = ("LICENSE-MIT", "LICENSE-APACHE", "NOTICE")
HEX40 = re.compile(r"[0-9a-f]{40}\Z")


def git_output(root: Path, *arguments: str) -> str | None:
    try:
        result = subprocess.run(
            ["git", *arguments], cwd=root, text=True, capture_output=True, check=True
        )
    except (OSError, subprocess.CalledProcessError):
        return None
    return result.stdout.strip()


def build_identity(root: Path, version: str) -> tuple[str, str | None, str | None]:
    """Only claim a commit when the entire source checkout is clean."""
    commit = git_output(root, "rev-parse", "--verify", "HEAD")
    status = git_output(root, "status", "--porcelain", "--untracked-files=all")
    if commit is None or not HEX40.fullmatch(commit) or status is None or status:
        return "local", None, None
    tags = git_output(root, "tag", "--points-at", "HEAD")
    if tags is not None and f"v{version}" in tags.splitlines():
        manifest_path = root / f"release-manifest-v{version}.json"
        try:
            manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
            raise ValueError(f"release tag has no readable manifest: {error}") from error
        source = manifest.get("commit")
        if (manifest.get("schema") != "sekirei.release-manifest.v1"
                or manifest.get("release") != f"v{version}"
                or not isinstance(source, str)
                or not HEX40.fullmatch(source)
                or git_output(root, "merge-base", source, "HEAD") != source):
            raise ValueError("release manifest source commit is invalid or not an ancestor")
        return "release", source, commit
    return "candidate", commit, commit


def wasm_build_environment(
    source: dict[str, str] | None = None,
    identity: tuple[str, str | None, str | None] = ("local", None, None),
) -> dict[str, str]:
    """Return a host-independent environment for the wasm32 release build."""
    environment = dict(os.environ if source is None else source)
    # The repository's local release profile uses target-cpu=native. Passing a
    # host CPU to wasm32 can produce a module that wasm-bindgen cannot process.
    # CI already clears this setting; keep the documented local build identical.
    environment["RUSTFLAGS"] = ""
    environment.pop("SEKIREI_WASM_SOURCE_COMMIT", None)
    environment.pop("SEKIREI_WASM_BUILD_COMMIT", None)
    environment.pop("SEKIREI_WASM_BUILD_PROVENANCE", None)
    provenance, source_commit, build_commit = identity
    if source_commit is not None and build_commit is not None:
        environment["SEKIREI_WASM_SOURCE_COMMIT"] = source_commit
        environment["SEKIREI_WASM_BUILD_COMMIT"] = build_commit
        environment["SEKIREI_WASM_BUILD_PROVENANCE"] = provenance
    return environment


def main() -> None:
    # Read Cargo metadata before invoking wasm-pack so the source identity is
    # bound to the exact package version that will be built.
    cargo_metadata = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=ROOT, text=True, capture_output=True, check=True,
    )
    packages = json.loads(cargo_metadata.stdout)["packages"]
    version = next(package["version"] for package in packages if package["name"] == "sekirei-wasm")
    identity = build_identity(ROOT, version)
    subprocess.run(
        [
            "wasm-pack",
            "build",
            "--target",
            "web",
            "--release",
            "--out-dir",
            "pkg",
            str(CRATE),
            "--locked",
        ],
        cwd=ROOT,
        env=wasm_build_environment(identity=identity),
        check=True,
    )

    if build_identity(ROOT, version) != identity:
        raise SystemExit("source checkout changed during the WASM build")

    package_json_path = PACKAGE / "package.json"
    package_json = json.loads(package_json_path.read_text(encoding="utf-8"))
    if package_json["version"] != version:
        raise SystemExit("wasm-pack package version differs from Cargo metadata")
    provenance, source_commit, build_commit = identity
    package_json["sekireiBuild"] = {
        "apiVersion": 4,
        "engineVersion": version,
        "sourceCommit": source_commit,
        "buildCommit": build_commit,
        "sourceCommitStatus": "known" if source_commit else "unknown",
        "provenance": provenance,
        "initialEvaluatorId": "material",
        "initialEvaluatorVersion": "material-v1",
        "initialEvaluatorWeightSha256": None,
    }
    (PACKAGE / "README.md").write_text(
        render_readme((CRATE / "README.md").read_text(encoding="utf-8"), version),
        encoding="utf-8",
    )
    packaged_files = package_json.setdefault("files", [])
    for filename in LEGAL_FILES:
        shutil.copyfile(ROOT / filename, PACKAGE / filename)
        if filename not in packaged_files:
            packaged_files.append(filename)
    package_json_path.write_text(
        json.dumps(package_json, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    errors = validate_package(
        package_json_path.read_bytes(),
        (PACKAGE / "README.md").read_text(encoding="utf-8"),
        f"sekirei-wasm-{version}.tgz",
    )
    if errors:
        raise SystemExit("invalid WASM package metadata: " + "; ".join(errors))


if __name__ == "__main__":
    main()
