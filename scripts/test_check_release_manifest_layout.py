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


def write_manifest(path: Path, version: str, status: str = "verified", wasm_status=None) -> None:
    document = {"release": f"v{version}", "publish": {"status": status}}
    if wasm_status is not None:
        document["webassembly"] = {"status": wasm_status}
    path.write_text(
        json.dumps(document),
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


def test_v051_archive_requires_verified_wasm_asset():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        write_manifest(root / "release-manifest-v0.5.2.json", "0.5.2")
        archive = root / "docs/releases/manifests"
        archive.mkdir(parents=True)
        older = archive / "release-manifest-v0.5.1.json"
        write_manifest(older, "0.5.1", wasm_status="planned")
        assert any("WASM release manifest is not verified" in error
                   for error in layout.check_layout(root, "0.5.2"))
        write_manifest(older, "0.5.1", wasm_status="verified")
        assert layout.check_layout(root, "0.5.2") == []


if __name__ == "__main__":
    test_accepts_current_root_manifest_and_verified_history()
    test_rejects_stale_root_manifest_and_unverified_archive()
    test_v051_archive_requires_verified_wasm_asset()
    print("PASS")
