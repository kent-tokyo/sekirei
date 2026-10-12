#!/usr/bin/env python3
"""Build the browser package and add the repository's legal notices."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
from pathlib import Path

from check_wasm_package_metadata import render_readme, validate_package


ROOT = Path(__file__).resolve().parents[1]
CRATE = ROOT / "crates" / "sekirei-wasm"
PACKAGE = CRATE / "pkg"
LEGAL_FILES = ("LICENSE-MIT", "LICENSE-APACHE", "NOTICE")


def wasm_build_environment(source: dict[str, str] | None = None) -> dict[str, str]:
    """Return a host-independent environment for the wasm32 release build."""
    environment = dict(os.environ if source is None else source)
    # The repository's local release profile uses target-cpu=native. Passing a
    # host CPU to wasm32 can produce a module that wasm-bindgen cannot process.
    # CI already clears this setting; keep the documented local build identical.
    environment["RUSTFLAGS"] = ""
    return environment


def main() -> None:
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
        env=wasm_build_environment(),
        check=True,
    )

    package_json_path = PACKAGE / "package.json"
    package_json = json.loads(package_json_path.read_text(encoding="utf-8"))
    version = package_json["version"]
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
