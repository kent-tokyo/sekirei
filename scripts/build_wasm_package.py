#!/usr/bin/env python3
"""Build the browser package and add the repository's legal notices."""

from __future__ import annotations

import json
import shutil
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
CRATE = ROOT / "crates" / "sekirei-wasm"
PACKAGE = CRATE / "pkg"
LEGAL_FILES = ("LICENSE-MIT", "LICENSE-APACHE", "NOTICE")


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
        check=True,
    )

    package_json_path = PACKAGE / "package.json"
    package_json = json.loads(package_json_path.read_text(encoding="utf-8"))
    packaged_files = package_json.setdefault("files", [])
    for filename in LEGAL_FILES:
        shutil.copyfile(ROOT / filename, PACKAGE / filename)
        if filename not in packaged_files:
            packaged_files.append(filename)
    package_json_path.write_text(
        json.dumps(package_json, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
