#!/usr/bin/env python3
import importlib.util
import json
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "release_manifest_layout", ROOT / "scripts/check_release_manifest_layout.py"
)
layout = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(layout)


def write_manifest(path: Path, version: str, status: str = "verified") -> None:
    path.write_text(
        json.dumps({"release": f"v{version}", "publish": {"status": status}}),
        encoding="utf-8",
    )


def test_accepts_current_root_manifest_and_verified_history():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        write_manifest(root / "release-manifest-v0.4.3.json", "0.4.3")
        archive = root / "docs/releases/manifests"
        archive.mkdir(parents=True)
        write_manifest(archive / "release-manifest-v0.4.2.json", "0.4.2")
        assert layout.check_layout(root, "0.4.3") == []


def test_rejects_stale_root_manifest_and_unverified_archive():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        write_manifest(root / "release-manifest-v0.4.2.json", "0.4.2")
        write_manifest(root / "release-manifest-v0.4.3.json", "0.4.3")
        archive = root / "docs/releases/manifests"
        archive.mkdir(parents=True)
        write_manifest(archive / "release-manifest-v0.4.1.json", "0.4.1", "planned")
        errors = layout.check_layout(root, "0.4.3")
        assert any("root release manifests" in error for error in errors)
        assert any("not verified" in error for error in errors)


if __name__ == "__main__":
    test_accepts_current_root_manifest_and_verified_history()
    test_rejects_stale_root_manifest_and_unverified_archive()
    print("PASS")
