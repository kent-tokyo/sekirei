#!/usr/bin/env python3
"""Check public release metadata without compiling or running the engine."""

from pathlib import Path
import sys
import tomllib


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
