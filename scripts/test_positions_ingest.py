#!/usr/bin/env python3
"""Ingestion CLI contracts using only newly generated synthetic positions.

TRAIN_BIN=/absolute/path/to/train python3 scripts/test_positions_ingest.py
Without TRAIN_BIN, only the benchmark helper tests run; CLI tests are skipped.
"""

import hashlib
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import benchmark_positions_ingest as benchmark


def training_corpus():
    """Tiny invented opening trajectories and outcomes, never actual game data."""
    sfens = (
        benchmark.STARTPOS,
        "lnsgkgsnl/1r5b1/ppppppppp/9/9/2P6/PP1PPPPPP/1B5R1/LNSGKGSNL w - 2",
        "lnsgkgsnl/1r5b1/pppppp1pp/6p2/9/2P6/PP1PPPPPP/1B5R1/LNSGKGSNL b - 3",
        "lnsgkgsnl/1r5b1/pppppp1pp/6p2/9/2P4P1/PP1PPPP1P/1B5R1/LNSGKGSNL w - 4",
        "lnsgkgsnl/1r5b1/p1pppp1pp/1p4p2/9/2P4P1/PP1PPPP1P/1B5R1/LNSGKGSNL b - 5",
        "lnsgkgsnl/1r5b1/p1pppp1pp/1p4p2/7P1/2P6/PP1PPPP1P/1B5R1/LNSGKGSNL w - 6",
    )
    rows = []
    for name, outcome in (("a", "black_wins"), ("b", "white_wins"), ("c", "draw")):
        for ply, sfen in enumerate(sfens, 1):
            row = benchmark.synthetic_record()
            row["sfen"] = sfen
            row["source"]["path"] = f"synthetic-game-{name}"
            row["source"]["ply"] = ply
            row["tags"]["side_to_move"] = "black" if ply % 2 else "white"
            row["game_result"] = {"outcome": outcome, "result_source": "synthetic-test"}
            rows.append(benchmark.record_bytes(row))
    return b"".join(rows)


class BenchmarkHelpersTest(unittest.TestCase):
    def test_platform_rss_units_are_normalized(self):
        self.assertEqual(benchmark.parse_max_rss_bytes(
            "  123456 maximum resident set size\n", "Darwin"), 123456)
        self.assertEqual(benchmark.parse_max_rss_bytes(
            "Maximum resident set size (kbytes): 1234\n", "Linux"), 1234 * 1024)
        with self.assertRaises(ValueError):
            benchmark.parse_max_rss_bytes("no timing evidence", "Linux")

    def test_process_logs_are_bounded_and_preserve_timing_tail(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            limit = benchmark.LOG_LIMIT
            command = [sys.executable, "-c",
                       f"import sys; sys.stdout.write('x' * {limit * 3}); "
                       f"sys.stderr.write('y' * {limit * 3} + 'TIMING-TAIL')"]
            code, elapsed, logs = benchmark.run_bounded(
                command, root / "stdout", root / "stderr", 10)
            self.assertEqual(code, 0)
            self.assertGreater(elapsed, 0)
            self.assertEqual(logs[0]["bytes"], limit * 3)
            self.assertTrue(logs[0]["truncated"])
            self.assertTrue(logs[1]["truncated"])
            self.assertTrue(logs[1]["tail"].endswith("TIMING-TAIL"))
            self.assertLess((root / "stdout").stat().st_size, limit * 2 + 100)
            self.assertLess((root / "stderr").stat().st_size, limit * 2 + 100)

    def test_synthetic_corpus_is_deterministic_and_pinned(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            first = benchmark.write_synthetic_corpus(root / "a.jsonl", 3)
            second = benchmark.write_synthetic_corpus(root / "b.jsonl", 3)
            self.assertEqual(first["sha256"], second["sha256"])
            self.assertEqual(first["bytes"], len(benchmark.record_bytes()) * 3)
            self.assertTrue(first["synthetic"])
            with self.assertRaises(FileExistsError):
                benchmark.write_synthetic_corpus(root / "a.jsonl", 1)


@unittest.skipUnless(os.environ.get("TRAIN_BIN"), "set TRAIN_BIN to enable ingestion CLI tests")
class PositionsIngestCliTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.binary = Path(os.environ["TRAIN_BIN"]).resolve(strict=True)

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="sekirei-ingest-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.counter = 0

    def invoke(self, data, mode="memory", extra=(), manifest=None, validation=None,
               explicit_manifest=True):
        self.counter += 1
        stem = f"run-{self.counter:02d}"
        source = self.root / f"{stem}.jsonl"
        source.write_bytes(data)
        output = self.root / f"{stem}.weights.bin"
        if manifest is None:
            manifest = output.with_suffix(".positions-manifest.json")
        command = [str(self.binary), "--positions", str(source),
                   "--positions-storage", mode, "--positions-audit-only",
                   "--output", str(output)]
        if explicit_manifest:
            command += ["--positions-manifest", str(manifest)]
        if validation is not None:
            validation_path = self.root / f"{stem}.validation.jsonl"
            validation_path.write_bytes(validation)
            command += ["--validation-positions", str(validation_path)]
        command += list(extra)
        stdout, stderr = self.root / f"{stem}.out", self.root / f"{stem}.err"
        code, _, _ = benchmark.run_bounded(command, stdout, stderr, timeout=60)
        self.assertFalse(output.exists(), "audit-only must not write weights")
        return code, manifest, stderr.read_text(), source

    def assert_success(self, result, mode):
        code, path, stderr, source = result
        self.assertEqual(code, 0, stderr)
        document = json.loads(path.read_text())
        self.assertEqual(document["schema"], "sekirei.positions-ingest.v1")
        self.assertIs(document["complete"], True)
        self.assertEqual(document["storage"], mode)
        self.assertEqual(Path(document["train"]["path"]), source)
        return document

    def test_storage_modes_preserve_full_report_and_rejected_evidence(self):
        good = benchmark.record_bytes()
        malformed = b"{broken json}\r\n"
        future = benchmark.synthetic_record()
        future["schema_version"] = 999999
        future = benchmark.record_bytes(future)
        invalid = benchmark.synthetic_record()
        invalid["sfen"] = "not a SFEN"
        invalid = benchmark.record_bytes(invalid)
        data = b"\n" + good + malformed + future + invalid + good.rstrip(b"\n")
        reports = []
        for mode in ("memory", "disk"):
            document = self.assert_success(self.invoke(data, mode), mode)
            self.assertIsNone(document["validation"])
            report = document["train"]["report"]
            self.assertEqual(report["sha256"], hashlib.sha256(data).hexdigest())
            self.assertEqual(report["input_bytes"], len(data))
            self.assertEqual(report["physical_lines"], 6)
            self.assertEqual(report["non_empty_lines"], 5)
            self.assertEqual(report["accepted"], 2)
            self.assertEqual(report["skipped"], 3)
            reasons = {"invalid_json_or_required_field": 1,
                       "unsupported_schema_version": 1, "invalid_sfen": 1}
            self.assertEqual(report["skipped_by_reason"], reasons)
            examples = report["rejected_examples"]
            self.assertEqual(len(examples), 3)
            offset = 1 + len(good)
            for line_number, row, example in zip((3, 4, 5), (malformed, future, invalid), examples):
                self.assertEqual(example["line_number"], line_number)
                self.assertEqual(example["byte_offset"], offset)
                self.assertEqual(example["row_sha256"], hashlib.sha256(row).hexdigest())
                self.assertIn(example["reason"], reasons)
                offset += len(row)
            reports.append(report)
        self.assertEqual(*reports)

    def test_line_endings_empty_and_blank_inputs(self):
        row = benchmark.record_bytes()
        cases = ((b"", 0, 0), (b"\n \t\r\n", 2, 0),
                 (row.rstrip(b"\n"), 1, 1),
                 (row.replace(b"\n", b"\r\n") + b"\r\n" + row.rstrip(b"\n"), 3, 2))
        for mode in ("memory", "disk"):
            for data, lines, accepted in cases:
                with self.subTest(mode=mode, data=data[:30]):
                    document = self.assert_success(self.invoke(data, mode), mode)
                    report = document["train"]["report"]
                    self.assertEqual(report["physical_lines"], lines)
                    self.assertEqual(report["accepted"], accepted)
                    self.assertEqual(report["non_empty_lines"], accepted)
                    self.assertEqual(report["skipped"], 0)
                    self.assertEqual(report["input_bytes"], len(data))
                    self.assertEqual(report["sha256"], hashlib.sha256(data).hexdigest())

    def test_default_manifest_and_audit_exit_before_weights_initialization(self):
        missing = str(self.root / "does-not-exist.bin")
        document = self.assert_success(self.invoke(
            benchmark.record_bytes(), explicit_manifest=False,
            extra=("--teacher-eval", "nnue", "--teacher-weights", missing,
                   "--init-weights", missing)), "memory")
        self.assertEqual(document["train"]["report"]["accepted"], 1)
        self.assertFalse(Path(missing).exists())

    def test_validation_report_and_hash_are_independent(self):
        train = benchmark.record_bytes()
        validation = b"\n" + train.replace(b"\n", b"\r\n")
        for mode in ("memory", "disk"):
            document = self.assert_success(self.invoke(
                train, mode, validation=validation,
                extra=("--positions-sha256", hashlib.sha256(train).hexdigest(),
                       "--validation-positions-sha256", hashlib.sha256(validation).hexdigest())), mode)
            self.assertEqual(document["validation"]["report"]["sha256"],
                             hashlib.sha256(validation).hexdigest())
            self.assertEqual(document["validation"]["report"]["input_bytes"], len(validation))
            self.assertEqual(document["validation"]["report"]["accepted"], 1)

    def test_strict_or_hash_failure_never_publishes_complete_manifest(self):
        good = benchmark.record_bytes()
        for mode in ("memory", "disk"):
            for data, extra, validation in (
                (good + b"broken\n", ("--strict-positions",), None),
                (good, ("--positions-sha256", "0" * 64), None),
                (good, ("--validation-positions-sha256", "0" * 64), good),
                (good, ("--strict-positions",), good + b"broken\n"),
            ):
                with self.subTest(mode=mode, extra=extra, validation=validation is not None):
                    code, manifest, stderr, _ = self.invoke(data, mode, extra, validation=validation)
                    self.assertNotEqual(code, 0, stderr)
                    self.assertFalse(manifest.exists(), stderr)

    def test_manifest_cannot_replace_input_or_alias(self):
        data = benchmark.record_bytes()
        for mode in ("memory", "disk"):
            for role in ("train", "validation"):
                for alias_kind in ("direct", "symlink", "hardlink"):
                    with self.subTest(mode=mode, role=role, alias=alias_kind):
                        self.counter += 1
                        source = self.root / f"protected-{self.counter}.jsonl"
                        source.write_bytes(data)
                        alias = source
                        if alias_kind != "direct":
                            alias = self.root / f"alias-{self.counter}.json"
                            if alias_kind == "symlink":
                                alias.symlink_to(source)
                            else:
                                os.link(source, alias)
                        output = self.root / f"alias-test-{self.counter}.bin"
                        command = [str(self.binary), "--positions", str(source),
                                   "--positions-storage", mode, "--positions-audit-only",
                                   "--positions-manifest", str(alias), "--output", str(output)]
                        if role == "validation":
                            separate_train = self.root / f"train-{self.counter}.jsonl"
                            separate_train.write_bytes(data)
                            command[2] = str(separate_train)
                            command += ["--validation-positions", str(source)]
                        stderr_path = self.root / f"alias-{self.counter}.err"
                        code, _, _ = benchmark.run_bounded(
                            command, self.root / f"alias-{self.counter}.out", stderr_path, 60)
                        self.assertNotEqual(code, 0, stderr_path.read_text())
                        self.assertEqual(source.read_bytes(), data)
                        self.assertEqual(alias.read_bytes(), data)
                        self.assertFalse(output.exists())


    def test_manifest_cannot_alias_input_through_missing_parent(self):
        for mode in ("memory", "disk"):
            source = self.root / f"protected-{mode}.jsonl"
            data = benchmark.record_bytes()
            source.write_bytes(data)
            alias = self.root / f"missing-{mode}" / ".." / source.name
            output = self.root / f"protected-{mode}.bin"
            command = [str(self.binary), "--positions", str(source),
                       "--positions-storage", mode, "--positions-audit-only",
                       "--positions-manifest", str(alias), "--output", str(output)]
            stderr = self.root / f"missing-{mode}.err"
            code, _, _ = benchmark.run_bounded(
                command, self.root / f"missing-{mode}.out", stderr, 60)
            self.assertNotEqual(code, 0, stderr.read_text())
            self.assertEqual(source.read_bytes(), data)
            self.assertFalse(output.exists())

    def test_manifest_cannot_alias_metadata_or_best_companions(self):
        data = benchmark.record_bytes()
        source = self.root / "companion-input.jsonl"
        source.write_bytes(data)
        sentinel = b"existing companion evidence must survive"
        for mode in ("memory", "disk"):
            for extension in ("meta.json", "best.bin", "best.meta.json"):
                for kind in ("direct", "symlink", "hardlink"):
                    with self.subTest(mode=mode, extension=extension, kind=kind):
                        self.counter += 1
                        output = self.root / f"companion-{self.counter}.bin"
                        companion = output.with_suffix(f".{extension}")
                        companion.write_bytes(sentinel)
                        alias = companion
                        if kind != "direct":
                            alias = self.root / f"companion-alias-{self.counter}.json"
                            if kind == "symlink":
                                alias.symlink_to(companion)
                            else:
                                os.link(companion, alias)
                        command = [str(self.binary), "--positions", str(source),
                                   "--positions-storage", mode, "--positions-audit-only",
                                   "--positions-manifest", str(alias), "--output", str(output)]
                        stderr = self.root / f"companion-{self.counter}.err"
                        code, _, _ = benchmark.run_bounded(
                            command, self.root / f"companion-{self.counter}.out", stderr, 60)
                        self.assertNotEqual(code, 0, stderr.read_text())
                        self.assertEqual(companion.read_bytes(), sentinel)
                        self.assertEqual(alias.read_bytes(), sentinel)
                        self.assertEqual(source.read_bytes(), data)
                        self.assertFalse(output.exists())

    def test_training_equivalence_and_cross_mode_mid_epoch_resume(self):
        source = self.root / "synthetic-training.jsonl"
        source.write_bytes(training_corpus())
        recipe = ["--positions", str(source), "--positions-sha256", benchmark.file_sha256(source),
                  "--epochs", "1", "--lr-schedule-epochs", "1", "--label-depth", "1",
                  "--teacher-eval", "material", "--seed", "42", "--shuffle-seed", "17",
                  "--source-cap", "5", "--validation-ratio", "0.25", "--side-balance",
                  "--wdl-lambda", "0.3", "--resume-checkpoint-every-games", "2"]

        def train(name, mode, extra=(), stopped=False):
            directory = self.root / name
            directory.mkdir()
            output = directory / "weights.bin"
            command = [str(self.binary), *recipe, "--positions-storage", mode,
                       "--output", str(output), *extra]
            code, _, _ = benchmark.run_bounded(
                command, directory / "stdout.log", directory / "stderr.log", 180)
            stderr = (directory / "stderr.log").read_text()
            self.assertEqual(code, 0, stderr)
            self.assertIn("18 positions loaded, 15 after source_cap=5", stderr)
            manifest = json.loads(output.with_suffix(".positions-manifest.json").read_text())
            self.assertEqual(manifest["train"]["report"]["accepted"], 18)
            self.assertEqual(manifest["train"]["report"]["skipped"], 0)
            self.assertEqual(manifest["storage"], mode)
            if stopped:
                self.assertFalse(output.exists())
                self.assertIn("stopping after requested atomic resume checkpoint", stderr)
            else:
                self.assertTrue(output.is_file(), stderr)
                self.assertTrue((directory / "weights.epoch1.bin").is_file(), stderr)
            return directory

        memory = train("full-memory", "memory")
        disk = train("full-disk", "disk")
        memory_meta = json.loads((memory / "weights.epoch1.meta.json").read_text())
        disk_meta = json.loads((disk / "weights.epoch1.meta.json").read_text())
        self.assertGreater(memory_meta["train_count"], 2)
        self.assertGreater(memory_meta["valid_count"], 0)
        self.assertEqual(memory_meta["train_count"] + memory_meta["valid_count"], 15)
        self.assertIsNotNone(memory_meta["train_wdl_component"])
        self.assertTrue(memory_meta["side_balance"])
        self.assertEqual(memory_meta["source_cap"], 5)
        self.assertEqual(memory_meta["shuffle_seed"], 17)
        for key in ("checkpoint_hash", "split_hash", "train_count", "valid_count",
                    "positions_sha256", "cache_hits", "cache_misses", "train_wdl_component"):
            self.assertEqual(memory_meta[key], disk_meta[key], key)
        for filename in ("weights.bin", "weights.epoch1.bin", "weights.epoch1.adam.json"):
            self.assertEqual(benchmark.file_sha256(memory / filename),
                             benchmark.file_sha256(disk / filename), filename)
        self.assertEqual((memory / "weights.bin").read_bytes(), (disk / "weights.bin").read_bytes())

        partial = train("partial-memory", "memory", ("--stop-after-resume-checkpoint",), True)
        partial_checkpoint = partial / "weights.resume.json"
        state = json.loads(partial_checkpoint.read_text())
        self.assertEqual(state["epoch_completed"], 0)
        self.assertEqual(state["next_game_index"], 2)
        self.assertEqual(state["optimizer"]["step"], 2)
        self.assertGreater(len(state["teacher_cache"]), 0)
        del state
        resumed = train("resumed-disk", "disk", ("--resume-checkpoint", str(partial_checkpoint)))
        resumed_meta = json.loads((resumed / "weights.epoch1.meta.json").read_text())
        self.assertEqual(resumed_meta["checkpoint_hash"], memory_meta["checkpoint_hash"])
        self.assertEqual(resumed_meta["split_hash"], memory_meta["split_hash"])
        for filename in ("weights.bin", "weights.epoch1.bin", "weights.epoch1.adam.json"):
            self.assertEqual(benchmark.file_sha256(memory / filename),
                             benchmark.file_sha256(resumed / filename), filename)
        self.assertEqual((memory / "weights.bin").read_bytes(),
                         (resumed / "weights.bin").read_bytes())


if __name__ == "__main__":
    unittest.main()
