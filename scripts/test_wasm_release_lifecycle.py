#!/usr/bin/env python3

import json
import hashlib
import io
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
from validate_release_manifest import validate
from finalize_wasm_release_manifest import finalize
from check_wasm_npm_distribution import REQUIRED, validate_release_tag


SOURCE = Path(__file__).parents[1] / "release-manifest-v0.5.0.json"


def future_manifest():
    doc = json.loads(SOURCE.read_text(encoding="utf-8"))
    doc["release"] = "v0.5.1"
    doc["packages"] = {name: "0.5.1" for name in doc["packages"]}
    wasm = doc["webassembly"]
    wasm["version"] = "0.5.1"
    wasm["asset"] = "sekirei-wasm-0.5.1.tgz"
    wasm["url"] = "https://github.com/kent-tokyo/sekirei/releases/download/v0.5.1/sekirei-wasm-0.5.1.tgz"
    return doc


class WasmReleaseLifecycleTests(unittest.TestCase):
    def test_legacy_v050_manifest_remains_valid(self):
        self.assertEqual(validate(json.loads(SOURCE.read_text())), [])

    def test_planned_v051_omits_digest_until_tag_is_frozen(self):
        doc = future_manifest()
        wasm = doc["webassembly"]
        wasm["status"] = "planned"
        del wasm["sha256"]
        del wasm["size"]
        self.assertEqual(validate(doc, allow_planned_publish=True), [])
        self.assertIn("webassembly.status", validate(doc))
        wasm["sha256"] = "0" * 64
        self.assertIn("webassembly.sha256", validate(doc, allow_planned_publish=True))

    def test_verified_v051_requires_final_asset_digest(self):
        doc = future_manifest()
        self.assertIn("webassembly.status", validate(doc))
        doc["webassembly"]["status"] = "verified"
        self.assertEqual(validate(doc), [])
        del doc["webassembly"]["sha256"]
        self.assertIn("webassembly.sha256", validate(doc))

    def test_finalizer_records_digest_only_after_checksum_agrees(self):
        doc = future_manifest()
        doc["webassembly"]["status"] = "planned"
        del doc["webassembly"]["sha256"]
        del doc["webassembly"]["size"]
        tag_commit = "b" * 40
        with tempfile.TemporaryDirectory() as directory:
            asset = Path(directory) / doc["webassembly"]["asset"]
            files = {name: name.encode() for name in REQUIRED}
            files["README.md"] = (
                "npm install sekirei-wasm@0.5.1\n"
                "npm install https://github.com/kent-tokyo/sekirei/releases/download/"
                "v0.5.1/sekirei-wasm-0.5.1.tgz\n"
            ).encode()
            files["sekirei_wasm.d.ts"] = b"startSfen searchCapabilities analyzePosition"
            files["package.json"] = json.dumps({
                "name": "sekirei-wasm", "version": "0.5.1",
                "sekireiBuild": {
                    "apiVersion": 4, "engineVersion": "0.5.1",
                    "sourceCommit": doc["commit"], "buildCommit": tag_commit,
                    "sourceCommitStatus": "known", "provenance": "release",
                    "initialEvaluatorId": "material",
                    "initialEvaluatorVersion": "material-v1",
                    "initialEvaluatorWeightSha256": None,
                },
            }).encode()
            with tarfile.open(asset, "w:gz") as archive:
                for name, data in files.items():
                    info = tarfile.TarInfo(f"package/{name}")
                    info.size = len(data)
                    archive.addfile(info, io.BytesIO(data))
            digest = hashlib.sha256(asset.read_bytes()).hexdigest()
            with self.assertRaises(ValueError):
                finalize(doc, asset, f"{'0' * 64}  {asset.name}\n", tag_commit)
            with self.assertRaisesRegex(ValueError, "buildCommit"):
                finalize(doc, asset, f"{digest}  {asset.name}\n", "c" * 40)
            wrong_source = json.loads(json.dumps(doc))
            wrong_source["commit"] = "d" * 40
            with self.assertRaisesRegex(ValueError, "sourceCommit"):
                finalize(wrong_source, asset, f"{digest}  {asset.name}\n", tag_commit)
            finalized = finalize(doc, asset, f"{digest}  {asset.name}\n", tag_commit)
            self.assertEqual(finalized["webassembly"]["status"], "verified")
            self.assertEqual(finalized["webassembly"]["sha256"], digest)
            self.assertEqual(finalized["webassembly"]["size"], len(asset.read_bytes()))
            self.assertEqual(validate(finalized), [])
            planned_path = Path(directory) / "planned.json"
            verified_path = Path(directory) / "verified.json"
            cargo_path = Path(directory) / "Cargo.toml"
            planned_path.write_text(json.dumps(doc))
            verified_path.write_text(json.dumps(finalized))
            cargo_path.write_text('[package]\nname = "sekirei-wasm"\nversion = "0.5.1"\n')
            self.assertEqual(validate_release_tag("v0.5.1", cargo_path, verified_path,
                                                  asset, planned_path)[1], [])
            self.assertIn("tagged planned release manifest is missing",
                          validate_release_tag("v0.5.1", cargo_path, verified_path,
                                               asset)[1])
            tampered = json.loads(json.dumps(doc))
            tampered["webassembly"]["sha256"] = digest
            planned_path.write_text(json.dumps(tampered))
            self.assertIn("tagged planned and post-Release verified manifests disagree",
                          validate_release_tag("v0.5.1", cargo_path, verified_path,
                                               asset, planned_path)[1])


if __name__ == "__main__":
    unittest.main()
