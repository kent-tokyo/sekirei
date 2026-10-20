#!/usr/bin/env python3

import io
import json
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from check_wasm_package_metadata import render_readme, read_archive, validate_package


TEMPLATE = """The prebuilt vX.Y.Z ES-module package is published.\n
npm install https://github.com/kent-tokyo/sekirei/releases/download/vX.Y.Z/sekirei-wasm-X.Y.Z.tgz\n
"""
COMMIT = "a" * 40


def package_metadata(version="0.3.66", provenance="release", commit=COMMIT, build_commit=COMMIT):
    return json.dumps({
        "version": version,
        "sekireiBuild": {
            "apiVersion": 4,
            "engineVersion": version,
            "sourceCommit": commit,
            "buildCommit": build_commit if commit else None,
            "sourceCommitStatus": "known" if commit else "unknown",
            "provenance": provenance,
            "initialEvaluatorId": "material",
            "initialEvaluatorVersion": "material-v1",
            "initialEvaluatorWeightSha256": None,
        },
    }).encode()


class WasmPackageMetadataTests(unittest.TestCase):
    def test_published_legacy_archive_without_provenance_remains_valid(self):
        legacy = b'{"version":"0.5.0"}'
        self.assertEqual(validate_package(
            legacy, render_readme(TEMPLATE, "0.5.0"), "sekirei-wasm-0.5.0.tgz"
        ), [])
        errors = validate_package(
            b'{"version":"0.5.1"}',
            render_readme(TEMPLATE, "0.5.1"),
            "sekirei-wasm-0.5.1.tgz",
        )
        self.assertTrue(any("sekireiBuild is missing" in error for error in errors))

    def test_rendered_readme_matches_package_and_asset(self):
        readme = render_readme(TEMPLATE, "0.3.66")
        self.assertEqual(
            validate_package(
                package_metadata(), readme, "sekirei-wasm-0.3.66.tgz"
            ),
            [],
        )

    def test_stale_readme_version_is_rejected(self):
        readme = render_readme(TEMPLATE, "0.3.62")
        errors = validate_package(
            package_metadata(), readme, "sekirei-wasm-0.3.66.tgz"
        )
        self.assertTrue(any("README versions" in error for error in errors))

    def test_stale_asset_name_is_rejected(self):
        errors = validate_package(
            package_metadata(),
            render_readme(TEMPLATE, "0.3.66"),
            "sekirei-wasm-0.3.62.tgz",
        )
        self.assertTrue(any("asset name" in error for error in errors))

    def test_archive_contents_are_checked(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "sekirei-wasm-0.3.66.tgz"
            with tarfile.open(path, "w:gz") as archive:
                for name, data in (
                    ("package/package.json", package_metadata()),
                    ("package/README.md", render_readme(TEMPLATE, "0.3.66").encode()),
                ):
                    info = tarfile.TarInfo(name)
                    info.size = len(data)
                    archive.addfile(info, io.BytesIO(data))
            package_json, readme = read_archive(path)
            self.assertEqual(validate_package(package_json, readme, path.name), [])

    def test_local_unknown_is_typed_and_release_manifest_requires_exact_commit(self):
        readme = render_readme(TEMPLATE, "0.3.66")
        asset = "sekirei-wasm-0.3.66.tgz"
        self.assertEqual(
            validate_package(package_metadata(provenance="local", commit=None), readme, asset),
            [],
        )
        manifest = {"release": "v0.3.66", "commit": COMMIT}
        self.assertEqual(validate_package(
            package_metadata(build_commit="c" * 40), readme, asset, manifest
        ), [])
        self.assertTrue(validate_package(
            package_metadata(provenance="local", commit=None), readme, asset, manifest
        ))
        self.assertTrue(validate_package(
            package_metadata(commit="b" * 40), readme, asset, manifest
        ))

    def test_inconsistent_or_short_source_commit_is_rejected(self):
        readme = render_readme(TEMPLATE, "0.3.66")
        asset = "sekirei-wasm-0.3.66.tgz"
        for commit, provenance in [("deadbeef", "release"), (None, "candidate")]:
            errors = validate_package(package_metadata(provenance=provenance, commit=commit), readme, asset)
            self.assertTrue(any("source/build commit" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
