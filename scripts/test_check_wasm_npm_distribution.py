#!/usr/bin/env python3

import hashlib
import io
import json
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from check_wasm_npm_distribution import (REQUIRED, archive_files, check_archive,
                                         compare_files, validate_release_tag)


VERSION = "0.5.0"
README = (f"npm install https://github.com/kent-tokyo/sekirei/releases/download/"
          f"v{VERSION}/sekirei-wasm-{VERSION}.tgz\n")


def package():
    files = {name: name.encode() for name in REQUIRED}
    files["package.json"] = json.dumps({"name": "sekirei-wasm", "version": VERSION}).encode()
    files["README.md"] = README.encode()
    files["sekirei_wasm.d.ts"] = b"startSfen searchCapabilities analyzePosition"
    return files


def archive(path, files):
    with tarfile.open(path, "w:gz") as tar:
        for name, data in files.items():
            info = tarfile.TarInfo(f"package/{name}")
            info.size = len(data)
            tar.addfile(info, io.BytesIO(data))


class NpmDistributionTests(unittest.TestCase):
    def test_valid_archive_and_identical_repacked_content(self):
        with tempfile.TemporaryDirectory() as directory:
            first, second = (Path(directory) / name for name in ("first.tgz", "second.tgz"))
            archive(first, package())
            archive(second, dict(reversed(list(package().items()))))
            files = archive_files(first)
            self.assertEqual(check_archive(files, VERSION), [])
            self.assertEqual(compare_files(files, archive_files(second)), [])

    def test_missing_types_notice_weights_and_wasm_differences_fail(self):
        files = package()
        del files["NOTICE"]
        del files["sekirei_wasm.d.ts"]
        files["nn_r6.bin"] = b"weights"
        errors = check_archive(files, VERSION)
        self.assertTrue(any("NOTICE" in error for error in errors))
        self.assertTrue(any("sekirei_wasm.d.ts" in error for error in errors))
        self.assertTrue(any("nn_r6.bin" in error for error in errors))
        other = package()
        other["sekirei_wasm_bg.wasm"] = b"changed"
        self.assertTrue(any("sekirei_wasm_bg.wasm" in error
                            for error in compare_files(package(), other)))

    def test_bad_metadata_and_stale_readme_fail(self):
        files = package()
        files["package.json"] = b'{"name":"other","version":"0.4.0"}'
        files["README.md"] = README.replace(VERSION, "0.4.0").encode()
        self.assertTrue(check_archive(files, VERSION))

    def test_new_packages_require_registry_install_example(self):
        files = package()
        errors = check_archive(files, "0.5.1")
        self.assertTrue(any("npm registry install example" in error for error in errors))
        files["README.md"] += b"\nnpm install sekirei-wasm@0.5.1\n"
        errors = check_archive(files, "0.5.1")
        self.assertFalse(any("npm registry install example" in error for error in errors))

    def test_tar_link_and_duplicate_fail(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "bad.tgz"
            with tarfile.open(path, "w:gz") as tar:
                info = tarfile.TarInfo("package/NOTICE")
                info.type = tarfile.SYMTYPE
                info.linkname = "../../outside"
                tar.addfile(info)
            with self.assertRaises(ValueError):
                archive_files(path)
            with tarfile.open(path, "w:gz") as tar:
                for data in (b"one", b"two"):
                    info = tarfile.TarInfo("package/NOTICE")
                    info.size = len(data)
                    tar.addfile(info, io.BytesIO(data))
            with self.assertRaises(ValueError):
                archive_files(path)

    def test_tag_cargo_manifest_and_digest_agree(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            asset = root / f"sekirei-wasm-{VERSION}.tgz"
            archive(asset, package())
            cargo = root / "Cargo.toml"
            cargo.write_text(f'[package]\nname = "sekirei-wasm"\nversion = "{VERSION}"\n')
            manifest = root / "manifest.json"
            manifest.write_text(json.dumps({"release": f"v{VERSION}", "webassembly": {
                "version": VERSION, "asset": asset.name,
                "sha256": hashlib.sha256(asset.read_bytes()).hexdigest(),
                "size": asset.stat().st_size}}))
            self.assertEqual(validate_release_tag(f"v{VERSION}", cargo, manifest, asset)[1], [])
            self.assertTrue(validate_release_tag("v0.4.0", cargo, manifest, asset)[1])
            self.assertTrue(validate_release_tag("v0.5.0-rc.1", cargo, manifest, asset)[1])
            asset.write_bytes(asset.read_bytes() + b"changed")
            self.assertTrue(validate_release_tag(f"v{VERSION}", cargo, manifest, asset)[1])


if __name__ == "__main__":
    unittest.main()
