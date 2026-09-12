#!/usr/bin/env python3
import importlib.util
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("metadata", ROOT / "scripts/check_release_metadata.py")
metadata = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(metadata)


def test_matching_tag_is_exact():
    assert metadata.has_matching_tag("0.3.36", ["v0.3.36", "v0.3.35"])
    assert not metadata.has_matching_tag("0.3.36", ["v0.3.35", "0.3.36"])


def test_release_manifest_path_is_version_specific(tmp_path):
    assert metadata.release_manifest_path("0.3.36", tmp_path).name == "release-manifest-v0.3.36.json"


if __name__ == "__main__":
    test_matching_tag_is_exact()
    from tempfile import TemporaryDirectory

    with TemporaryDirectory() as directory:
        test_release_manifest_path_is_version_specific(Path(directory))
    print("PASS")
