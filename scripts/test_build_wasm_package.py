#!/usr/bin/env python3

import unittest
import json
import sys
import tempfile
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).parent))
from build_wasm_package import build_identity, wasm_build_environment


class WasmBuildEnvironmentTests(unittest.TestCase):
    def test_native_host_flags_are_cleared_without_losing_other_variables(self):
        environment = wasm_build_environment(
            {"PATH": "/toolchain/bin", "RUSTFLAGS": "-C target-cpu=native"}
        )

        self.assertEqual(environment["PATH"], "/toolchain/bin")
        self.assertEqual(environment["RUSTFLAGS"], "")

    def test_identity_requires_clean_checkout_and_matching_release_tag(self):
        commit = "a" * 40
        source = "b" * 40
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "release-manifest-v0.5.0.json").write_text(json.dumps({
                "schema": "sekirei.release-manifest.v1",
                "release": "v0.5.0",
                "commit": source,
            }))
            with patch("build_wasm_package.git_output", side_effect=[commit, "", "v0.5.0", source]):
                self.assertEqual(build_identity(root, "0.5.0"), ("release", source, commit))
            with patch("build_wasm_package.git_output", side_effect=[commit, "", "v0.5.0", None]):
                with self.assertRaisesRegex(ValueError, "not an ancestor"):
                    build_identity(root, "0.5.0")
        with patch("build_wasm_package.git_output", side_effect=[commit, "", "v0.4.0"]):
            self.assertEqual(build_identity(Path("."), "0.5.0"), ("candidate", commit, commit))
        with patch("build_wasm_package.git_output", side_effect=[commit, " M src/lib.rs"]):
            self.assertEqual(build_identity(Path("."), "0.5.0"), ("local", None, None))
        with patch("build_wasm_package.git_output", side_effect=[None, None]):
            self.assertEqual(build_identity(Path("."), "0.5.0"), ("local", None, None))

    def test_environment_replaces_untrusted_inherited_provenance(self):
        inherited = {
            "SEKIREI_WASM_SOURCE_COMMIT": "f" * 40,
            "SEKIREI_WASM_BUILD_COMMIT": "f" * 40,
            "SEKIREI_WASM_BUILD_PROVENANCE": "release",
        }
        self.assertNotIn(
            "SEKIREI_WASM_SOURCE_COMMIT", wasm_build_environment(inherited)
        )
        candidate = wasm_build_environment(inherited, ("candidate", "a" * 40, "a" * 40))
        self.assertEqual(candidate["SEKIREI_WASM_SOURCE_COMMIT"], "a" * 40)
        self.assertEqual(candidate["SEKIREI_WASM_BUILD_COMMIT"], "a" * 40)
        self.assertEqual(candidate["SEKIREI_WASM_BUILD_PROVENANCE"], "candidate")


if __name__ == "__main__":
    unittest.main()
