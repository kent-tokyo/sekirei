#!/usr/bin/env python3

import unittest

from build_wasm_package import wasm_build_environment


class WasmBuildEnvironmentTests(unittest.TestCase):
    def test_native_host_flags_are_cleared_without_losing_other_variables(self):
        environment = wasm_build_environment(
            {"PATH": "/toolchain/bin", "RUSTFLAGS": "-C target-cpu=native"}
        )

        self.assertEqual(environment["PATH"], "/toolchain/bin")
        self.assertEqual(environment["RUSTFLAGS"], "")


if __name__ == "__main__":
    unittest.main()
