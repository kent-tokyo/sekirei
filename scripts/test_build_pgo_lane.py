#!/usr/bin/env python3
"""Unit tests for the fail-closed compiler PGO lane."""

import argparse
import subprocess
import sys
from tempfile import TemporaryDirectory
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))

from build_pgo_lane import (  # noqa: E402
    cargo_build,
    encoded_rustflags,
    failure_record,
    parse_threads,
    profile_output_pattern,
    require_clean_source,
)


class PgoLaneTests(unittest.TestCase):
    def test_threads_require_positive_unique_integers(self):
        self.assertEqual(parse_threads("4,8"), [4, 8])
        for value in ("", "4,4", "0,8", "four,8"):
            with self.subTest(value=value), self.assertRaises(argparse.ArgumentTypeError):
                parse_threads(value)

    def test_encoded_flags_preserve_native_cpu_and_do_not_depend_on_config_precedence(self):
        flags = encoded_rustflags(
            "profile-generate=/tmp/raw",
            "llvm-args=-pgo-warn-missing-function",
        )
        self.assertEqual(
            flags.split("\x1f"),
            [
                "-C",
                "target-cpu=native",
                "-C",
                "profile-generate=/tmp/raw",
                "-C",
                "llvm-args=-pgo-warn-missing-function",
            ],
        )

    def test_dirty_source_is_rejected_before_any_build(self):
        with patch("build_pgo_lane.git_output", return_value=" M scripts/x.py"):
            with self.assertRaisesRegex(ValueError, "clean source"):
                require_clean_source()

    def test_native_build_uses_cargo_target_dir_without_a_host_subdirectory(self):
        target_dir = Path("/tmp/pgo-target")
        expected = target_dir / "release" / "sekirei"
        with patch("build_pgo_lane.checked"), patch.object(Path, "is_file", return_value=True):
            self.assertEqual(cargo_build(target_dir, None), expected)

    def test_profile_pattern_is_absolute_for_binary_relative_usi_child(self):
        with TemporaryDirectory() as tmp:
            self.assertEqual(
                profile_output_pattern(Path(tmp) / "raw-profdata"),
                str((Path(tmp) / "raw-profdata" / "train-%m.profraw").resolve()),
            )

    def test_profile_pattern_keeps_an_absolute_receipt_root_absolute(self):
        root = Path("/private/tmp/pgo-receipt").resolve()
        self.assertEqual(
            profile_output_pattern(root / "raw-profdata"),
            str(root / "raw-profdata" / "train-%m.profraw"),
        )

    def test_failed_command_preserves_captured_diagnostics(self):
        error = subprocess.CalledProcessError(
            101,
            ["cargo", "build"],
            output="compiler stdout",
            stderr="compiler stderr",
        )
        self.assertEqual(
            failure_record(error),
            {
                "type": "CalledProcessError",
                "message": "Command '['cargo', 'build']' returned non-zero exit status 101.",
                "command": ["cargo", "build"],
                "returncode": 101,
                "stdout": "compiler stdout",
                "stderr": "compiler stderr",
            },
        )


if __name__ == "__main__":
    unittest.main()
