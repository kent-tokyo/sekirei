#!/usr/bin/env python3
"""Unit tests for the fail-closed compiler PGO lane."""

import argparse
import json
import subprocess
import sys
from tempfile import TemporaryDirectory
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))

import build_pgo_lane as lane

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


class PgoReceiptTests(unittest.TestCase):
    def test_inventory_rejects_malformed_incomplete_and_duplicate_paths(self):
        names = ["Cargo.toml", "Cargo.lock", ".cargo/config.toml", "crates/core/src/lib.rs", "crates/core/build.rs", "scripts/build_pgo_lane.py", "scripts/nps_threads.py"]
        valid = "\0".join(names) + "\0"
        self.assertEqual(lane.compiler_input_paths(valid), sorted(names))
        for raw in ("", valid[:-1], "Cargo.toml\0", valid + "../outside.rs\0", valid + "/absolute.rs\0", valid + "Cargo.toml\0", valid + "crates//x.rs\0"):
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                lane.compiler_input_paths(raw)

    def test_source_snapshot_binds_lock_config_build_script_and_itself(self):
        names = ["Cargo.toml", "Cargo.lock", ".cargo/config.toml", "crates/core/src/lib.rs", "crates/core/build.rs", "scripts/build_pgo_lane.py", "scripts/nps_threads.py"]
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            for name in names:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(name)
            def git(*args):
                return "" if args[0] == "status" else "commit1"
            with patch.object(lane, "ROOT", root), patch.object(lane, "git_output", side_effect=git), patch.object(lane, "checked", return_value="\0".join(names) + "\0"):
                snapshot = lane.require_clean_source()
                self.assertEqual(set(snapshot["tracked_inputs"]), set(names))
                (root / "crates/core/build.rs").write_text("changed while git pretends clean")
                with self.assertRaisesRegex(ValueError, "source HEAD or tracked"):
                    lane.validate_inputs(snapshot, {})

    def test_head_or_bound_file_changes_fail_closed(self):
        baseline = {"commit": "a", "tracked_inputs": {}}
        with patch.object(lane, "require_clean_source", return_value={"commit": "b", "tracked_inputs": {}}):
            with self.assertRaisesRegex(ValueError, "source HEAD"):
                lane.validate_inputs(baseline, {})
        with TemporaryDirectory() as tmp:
            path = Path(tmp) / "bound"
            path.write_text("before")
            manifest = {"bound_inputs": {}}
            lane.bind_input(manifest, "corpus", path)
            path.write_text("after")
            with patch.object(lane, "require_clean_source", return_value=baseline), self.assertRaisesRegex(ValueError, "bound input changed"):
                lane.validate_inputs(baseline, manifest["bound_inputs"])

    def test_build_and_training_profiles_are_isolated_and_filter_is_strict(self):
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            env = lane.build_env(root / "target", compiler_raw_dir=root / "compiler")
            self.assertEqual(env["LLVM_PROFILE_FILE"], str((root / "compiler/build-%m-%p.profraw").resolve()))
            (root / "build-123.profraw").write_bytes(b"build script")
            with self.assertRaisesRegex(ValueError, "no train"):
                lane.training_profiles(root)
            train = root / "train-456.profraw"
            train.write_bytes(b"engine")
            self.assertEqual(lane.training_profiles(root), [train])
            train.write_bytes(b"")
            with self.assertRaisesRegex(ValueError, "invalid training"):
                lane.training_profiles(root)
            train.unlink()
            (root / "train-link.profraw").symlink_to(root / "build-123.profraw")
            with self.assertRaisesRegex(ValueError, "non-symlink"):
                lane.training_profiles(root)

    def test_successful_compiler_warning_is_preserved(self):
        with TemporaryDirectory() as tmp:
            receipt = Path(tmp) / "compile.json"
            process = subprocess.CompletedProcess(["cargo"], 0, "built", "warning: profile missing")
            with patch.object(lane.subprocess, "run", return_value=process):
                self.assertEqual(lane.checked(["cargo"], receipt=receipt), "built")
            saved = json.loads(receipt.read_text())
            self.assertEqual(saved["stderr"], "warning: profile missing")
            self.assertEqual(saved["returncode"], 0)

    def test_training_count_and_rows_require_complete_bound_material_work(self):
        from nps_threads import effective_positions_sha256
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            binary, corpus, output = root / "engine", root / "positions", root / "rows.jsonl"
            binary.write_bytes(b"fake")
            corpus.write_text("# comment\nstartpos\n")
            with self.assertRaisesRegex(ValueError, "fewer positions"):
                lane.selected_training_positions(corpus, 2)
            row = {"threads": 8, "position": 0, "repeat": 0, "nodes": 1,
                   "process_shutdown": {"exitcode": 0, "killed": False, "reaped": True},
                   "provenance": {"evaluator": {"kind": "material-only", "path": None, "sha256": None},
                       "engine_sha256": lane.sha256_file(binary), "positions_file_sha256": lane.sha256_file(corpus),
                       "effective_positions_sha256": effective_positions_sha256(["startpos"]), "effective_position_count": 1,
                       "options": [*lane.TRAINING_OPTIONS, "Threads=8", "Hash=64"]}}
            output.write_text(json.dumps(row) + "\n")
            self.assertEqual(lane.validate_training_output(output, binary, corpus, [8], 1)["validated_rows"], 1)
            for text in ("", "not JSON", "[]", json.dumps(row) + "\n" + json.dumps(row), json.dumps({**row, "nodes": 0}), json.dumps({**row, "position": 1}), json.dumps({**row, "process_shutdown": {"exitcode": 0, "killed": True, "reaped": True}}), json.dumps({**row, "provenance": {**row["provenance"], "evaluator": {"kind": "halfkp"}}})):
                output.write_text(text + "\n" if text else "")
                with self.subTest(text=text), self.assertRaises(ValueError):
                    lane.validate_training_output(output, binary, corpus, [8], 1)

    def test_partial_manifest_keeps_completed_binary_and_failed_phase(self):
        with TemporaryDirectory() as tmp:
            root = Path(tmp)
            corpus, tool = root / "positions", root / "profdata"
            corpus.write_text("startpos\n")
            tool.write_text("fake tool")
            output = root / "lane"
            source = {"commit": "frozen", "tracked_inputs": {}}
            calls = []
            def fake_build(target_dir, target, *flags, **kwargs):
                calls.append(kwargs)
                if len(calls) == 2:
                    raise ValueError("synthetic generation failure")
                path = target_dir / "release/sekirei"
                path.parent.mkdir(parents=True)
                path.write_bytes(b"native")
                return path
            argv = ["build_pgo_lane.py", "--output", str(output), "--training-positions", str(corpus), "--count", "1"]
            with patch.object(sys, "argv", argv), patch.object(lane, "require_clean_source", return_value=source), patch.object(lane, "llvm_profdata", return_value=tool), patch.object(lane, "rust_host", return_value="test-host"), patch.object(lane, "checked", return_value="fake version"), patch.object(lane, "cargo_build", side_effect=fake_build), self.assertRaisesRegex(ValueError, "synthetic"):
                lane.main()
            saved = json.loads((output / "manifest.json").read_text())
            self.assertEqual(saved["status"], "incomplete")
            self.assertEqual([phase["status"] for phase in saved["phases"]], ["complete", "incomplete"])
            self.assertIn("native", saved["binaries"])
            self.assertEqual(calls[1]["compiler_raw_dir"], (output / "compiler-profdata").resolve())


if __name__ == "__main__":
    unittest.main()
