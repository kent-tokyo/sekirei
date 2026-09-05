#!/usr/bin/env python3
"""Check public release metadata without compiling or running the engine."""

import json
from pathlib import Path
import re
import sys
import tomllib

from validate_release_manifest import validate as validate_release_manifest


ROOT = Path(__file__).resolve().parent.parent
MANIFESTS = {
    "sekirei-core": ROOT / "crates/sekirei-core/Cargo.toml",
    "sekirei-bench": ROOT / "crates/sekirei-bench/Cargo.toml",
    "sekirei-csa": ROOT / "crates/sekirei-csa/Cargo.toml",
    "sekirei-match-runner": ROOT / "crates/sekirei-match-runner/Cargo.toml",
    "sekirei-train": ROOT / "crates/sekirei-train/Cargo.toml",
    "sekirei": ROOT / "crates/sekirei-usi/Cargo.toml",
}


def main() -> int:
    errors: list[str] = []
    packages = {name: tomllib.loads(path.read_text())["package"] for name, path in MANIFESTS.items()}
    versions = {pkg["version"] for pkg in packages.values()}
    licenses = {pkg.get("license") for pkg in packages.values()}
    if len(versions) != 1:
        errors.append(f"crate versions disagree: {sorted(versions)}")
    if licenses != {"MIT OR Apache-2.0"}:
        errors.append(f"crate licenses disagree: {sorted(licenses)}")

    if expected := next(iter(versions), None):
        for name, package in packages.items():
            dependency = package.get("dependencies", {}).get("sekirei-core")
            if isinstance(dependency, dict) and dependency.get("version") != expected:
                errors.append(
                    f"{name} sekirei-core dependency version disagrees: "
                    f"{dependency.get('version')!r} != {expected!r}"
                )

    lock = tomllib.loads((ROOT / "Cargo.lock").read_text())
    lock_versions = {
        package["name"]: package["version"]
        for package in lock.get("package", [])
        if package["name"] in MANIFESTS
    }
    expected = next(iter(versions), None)
    missing = sorted(set(MANIFESTS) - set(lock_versions))
    if missing:
        errors.append(f"Cargo.lock is missing workspace packages: {missing}")
    mismatched = sorted(name for name in MANIFESTS if lock_versions.get(name) != expected)
    if mismatched:
        errors.append(f"Cargo.lock versions disagree for: {mismatched}")

    if expected:
        changelog = (ROOT / "CHANGELOG.md").read_text()
        heading = rf"^## \[{re.escape(expected)}\](?:\s|$)"
        if not re.search(heading, changelog, re.MULTILINE):
            errors.append(f"CHANGELOG.md is missing a release heading for {expected}")
        public_docs = {
            "README.md": (ROOT / "README.md").read_text(),
            "README_ja.md": (ROOT / "README_ja.md").read_text(),
        }
        for filename, contents in public_docs.items():
            if f"`{expected}`" not in contents:
                errors.append(f"{filename} is missing the current release version {expected}")

        release_manifest = ROOT / f"release-manifest-v{expected}.json"
        if release_manifest.is_file():
            try:
                release_manifest_doc = json.loads(release_manifest.read_text())
            except (OSError, ValueError) as exc:
                errors.append(f"current release manifest is unreadable: {exc}")
            else:
                manifest_errors = validate_release_manifest(release_manifest_doc)
                if manifest_errors:
                    errors.append(
                        "current release manifest is invalid: " + ", ".join(manifest_errors)
                    )

    required_files = ("LICENSE-MIT", "LICENSE-APACHE", "NOTICE", "NNUE-LICENSE.md")
    for filename in required_files:
        if not (ROOT / filename).is_file():
            errors.append(f"missing public license file: {filename}")
    notice = (ROOT / "NOTICE").read_text() if (ROOT / "NOTICE").is_file() else ""
    if "Kentaro Tanabe" not in notice or "Sekirei" not in notice:
        errors.append("NOTICE must contain Sekirei and Kentaro Tanabe attribution")

    if errors:
        for error in errors:
            print(f"error: {error}", file=sys.stderr)
        return 1
    print(f"release metadata OK: version={expected}, license=MIT OR Apache-2.0, crates={len(MANIFESTS)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
