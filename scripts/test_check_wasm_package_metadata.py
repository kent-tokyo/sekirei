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


class WasmPackageMetadataTests(unittest.TestCase):
    def test_rendered_readme_matches_package_and_asset(self):
        readme = render_readme(TEMPLATE, "0.3.66")
        self.assertEqual(
            validate_package(
                b'{"version":"0.3.66"}', readme, "sekirei-wasm-0.3.66.tgz"
            ),
            [],
        )

    def test_stale_readme_version_is_rejected(self):
        readme = render_readme(TEMPLATE, "0.3.62")
        errors = validate_package(
            b'{"version":"0.3.66"}', readme, "sekirei-wasm-0.3.66.tgz"
        )
        self.assertTrue(any("README versions" in error for error in errors))

    def test_stale_asset_name_is_rejected(self):
        errors = validate_package(
            b'{"version":"0.3.66"}',
            render_readme(TEMPLATE, "0.3.66"),
            "sekirei-wasm-0.3.62.tgz",
        )
        self.assertTrue(any("asset name" in error for error in errors))

    def test_archive_contents_are_checked(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "sekirei-wasm-0.3.66.tgz"
            with tarfile.open(path, "w:gz") as archive:
                for name, data in (
                    ("package/package.json", json.dumps({"version": "0.3.66"}).encode()),
                    ("package/README.md", render_readme(TEMPLATE, "0.3.66").encode()),
                ):
                    info = tarfile.TarInfo(name)
                    info.size = len(data)
                    archive.addfile(info, io.BytesIO(data))
            package_json, readme = read_archive(path)
            self.assertEqual(validate_package(package_json, readme, path.name), [])


if __name__ == "__main__":
    unittest.main()
