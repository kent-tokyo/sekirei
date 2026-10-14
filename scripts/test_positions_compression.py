#!/usr/bin/env python3
"""Compressed positions contracts; fixtures use no external data or C dependencies.

TRAIN_BIN=/absolute/path/to/train POSITIONS_CODECS=gzip,zstd \
    python3 -B scripts/test_positions_compression.py

POSITIONS_CODECS explicitly lists the enabled positions-gzip/positions-zstd
features; empty means neither. Disabled formats must fail with a feature error.
All training uses six invented opening records and material teacher depth one.
"""

import gzip
import hashlib
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import benchmark_positions_ingest as benchmark
from test_positions_ingest import training_corpus

SUFFIXES = {"plain": ".jsonl", "gzip": ".jsonl.gz", "zstd": ".jsonl.zst"}

# Generated from benchmark.record_bytes()*2 with ruzstd =0.9.1,
# default-features=false, features=["std", "hash"], using
# encoding::compress_to_vec(raw.as_slice(), encoding::CompressionLevel::Fastest).
# StreamingDecoder roundtrip and its calculated/data checksum were independently
# checked during generation (both 2499585070). First block is compressed, not raw.
# Generation requires only Pure Rust; test execution requires only Python stdlib.
ZSTD_COMPRESSED_FRAME_HEX = (
    "28b52ffd04382d09004c10007b22736368656d615f76657273696f6e223a31312c227366656e223a226c6e73"
    "676b67736e6c2f31723562312f7070707070707070702f392f392f392f5050505050505050502f3142355231"
    "2f4c4e53474b47534e4c2062202d2031222c2274616773223a7b227068617365223a226f70656e696e67222c"
    "22736964655f746f5f6d6f7665223a22626c61636b222c22696e5f636865636b223a66616c73652c22686173"
    "5f636170747572657d2c22736f75726365223a7b226b696e64223a226669782c2270617468223a2273796e74"
    "68657469632d706f736974696f6e732d696e67657374222c22706c79223a317d2c226f62736572766174696f"
    "6e73223a5b5d7d0a03a810fe1f18271f10064af401108281c4ffffbf1f41c3c47829b02e731f2ea4fc94"
)
ZSTD_COMPRESSED_FRAME_SHA256 = "febc52ba78496ddfa69b4b487d49bcf9bd18bdeb41a172362e71bee7c92bc018"
ZSTD_COMPRESSED_DECODED_SHA256 = "152f5da62ea9b1acd355f3e3c0f913b4a0dcd01bc1b30f4fa3db75c0b92cb77d"


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def zstd_raw_frame(data):
    """Encode one bounded raw block, not a compression-performance fixture.

    Format reference: facebook/zstd doc/zstd_compression_format.md, frame header
    and block sections. Single segment, 4-byte content size, no dictionary or
    checksum; last raw block. A separate fixed frame tests compressed blocks
    and checksum verification. No third-party encoder runs in this test suite.
    """
    if len(data) > 128 * 1024:
        raise ValueError("raw test frame must fit one 128 KiB block")
    header = b"\x28\xb5\x2f\xfd\xa0" + len(data).to_bytes(4, "little")
    return header + ((len(data) << 3) | 1).to_bytes(3, "little") + data


def encode(data, codec):
    if codec == "plain":
        return data
    if codec == "gzip":
        return gzip.compress(data, mtime=0)
    if codec == "zstd":
        return zstd_raw_frame(data)
    raise ValueError(f"unknown codec: {codec}")


def decoded_report(report):
    """Remove only physical transport identity, retaining every decoded field."""
    return {key: value for key, value in report.items()
            if key not in {"sha256", "input_bytes", "compression"}}


def enabled_codecs():
    codecs = {value.strip() for value in os.environ.get("POSITIONS_CODECS", "").split(",")
              if value.strip()}
    unknown = codecs - {"gzip", "zstd"}
    if unknown:
        raise ValueError(f"unknown POSITIONS_CODECS entries: {sorted(unknown)}")
    return codecs


class CompressionFixtureTest(unittest.TestCase):
    def test_generated_fixtures_are_bounded_and_deterministic(self):
        data = benchmark.record_bytes()
        self.assertEqual(gzip.decompress(encode(data, "gzip")), data)
        self.assertEqual(encode(data, "gzip"), encode(data, "gzip"))
        frame = zstd_raw_frame(data)
        self.assertEqual(frame[:5], b"\x28\xb5\x2f\xfd\xa0")
        self.assertEqual(int.from_bytes(frame[5:9], "little"), len(data))
        self.assertEqual(int.from_bytes(frame[9:12], "little"), (len(data) << 3) | 1)
        self.assertEqual(frame[12:], data)
        self.assertEqual(zstd_raw_frame(b"")[-3:], b"\x01\x00\x00")
        with self.assertRaises(ValueError):
            zstd_raw_frame(b"x" * (128 * 1024 + 1))

    def test_fixed_compressed_fixture_is_pinned(self):
        frame = bytes.fromhex(ZSTD_COMPRESSED_FRAME_HEX)
        self.assertEqual(sha256(frame), ZSTD_COMPRESSED_FRAME_SHA256)
        self.assertEqual(sha256(benchmark.record_bytes() * 2), ZSTD_COMPRESSED_DECODED_SHA256)
        self.assertEqual(frame[:4], b"\x28\xb5\x2f\xfd")
        self.assertTrue(frame[4] & 4, "fixture must contain a content checksum")
        # The pinned descriptor has a window byte, no dictionary/content size.
        self.assertEqual(frame[4], 4)
        self.assertEqual((int.from_bytes(frame[6:9], "little") >> 1) & 3, 2)


@unittest.skipUnless(os.environ.get("TRAIN_BIN"), "set TRAIN_BIN to run compression CLI tests")
class PositionsCompressionCliTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.binary = Path(os.environ["TRAIN_BIN"]).resolve(strict=True)
        cls.codecs = enabled_codecs()

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="sekirei-compression-test-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.counter = 0

    def invoke(self, raw, mode, suffix=".jsonl", extra=(), validation=None,
               previous_manifest=None, audit=True):
        self.counter += 1
        directory = self.root / f"run-{self.counter:03d}"
        directory.mkdir()
        source = directory / f"positions{suffix}"
        source.write_bytes(raw)
        output = directory / "weights.bin"
        manifest = output.with_suffix(".positions-manifest.json")
        if previous_manifest is not None:
            manifest.write_bytes(previous_manifest)
        command = [str(self.binary), "--positions", str(source),
                   "--positions-storage", mode, "--output", str(output)]
        if audit:
            command.append("--positions-audit-only")
        if validation is not None:
            validation_bytes, validation_suffix = validation
            validation_path = directory / f"validation{validation_suffix}"
            validation_path.write_bytes(validation_bytes)
            command += ["--validation-positions", str(validation_path)]
        command += list(extra)
        code, _, _ = benchmark.run_bounded(
            command, directory / "stdout.log", directory / "stderr.log", 180 if not audit else 60)
        if audit:
            self.assertFalse(output.exists(), "audit-only wrote weights")
        return {"code": code, "directory": directory, "source": source, "storage": mode,
                "output": output, "manifest": manifest,
                "stderr": (directory / "stderr.log").read_text()}

    def assert_success(self, result, codec, raw, decoded):
        self.assertEqual(result["code"], 0, result["stderr"])
        document = json.loads(result["manifest"].read_text())
        self.assertEqual(document["schema"], "sekirei.positions-ingest.v1")
        self.assertIs(document["complete"], True)
        self.assertEqual(document["storage"], result["storage"])
        report = document["train"]["report"]
        self.assert_report_hashes(report, codec, raw, decoded)
        return document

    def assert_report_hashes(self, report, codec, raw, decoded):
        self.assertEqual(report["compression"], codec)
        self.assertEqual(report["sha256"], sha256(raw))
        self.assertEqual(report["input_bytes"], len(raw))
        self.assertEqual(report["decoded_sha256"], sha256(decoded))
        self.assertEqual(report["decoded_bytes"], len(decoded))

    def assert_ingest_failure(self, result, previous=None):
        self.assertNotEqual(result["code"], 0, result["stderr"])
        self.assertFalse(result["output"].exists(), result["stderr"])
        if previous is None:
            self.assertFalse(result["manifest"].exists(), result["stderr"])
        else:
            self.assertEqual(result["manifest"].read_bytes(), previous)

    def test_disabled_features_fail_explicitly_with_magic_or_extension(self):
        data = benchmark.record_bytes()
        for codec in sorted({"gzip", "zstd"} - self.codecs):
            for mode in ("memory", "disk"):
                for raw, suffix in ((encode(data, codec), ".jsonl"),
                                    (encode(data, codec), SUFFIXES[codec]),
                                    (data, SUFFIXES[codec])):
                    with self.subTest(codec=codec, mode=mode, suffix=suffix):
                        result = self.invoke(raw, mode, suffix)
                        self.assert_ingest_failure(result)
                        self.assertIn(f"positions-{codec}", result["stderr"])

    def test_all_codecs_preserve_decoded_audit_and_rejected_offsets(self):
        row = benchmark.synthetic_record()
        row["source"]["path"] = "synthetic-駒"
        good = json.dumps(row, ensure_ascii=False, separators=(",", ":")).encode() + b"\r\n"
        malformed = b"{broken json}\r\n"
        future = benchmark.synthetic_record()
        future["schema_version"] = 999999
        future = benchmark.record_bytes(future)
        decoded = b"\r\n" + good + malformed + future + benchmark.record_bytes().rstrip(b"\n")
        baseline = None
        for codec in ("plain", *sorted(self.codecs)):
            raw = encode(decoded, codec)
            for mode in ("memory", "disk"):
                with self.subTest(codec=codec, mode=mode):
                    result = self.invoke(raw, mode, SUFFIXES[codec],
                                         extra=("--positions-sha256", sha256(raw)))
                    report = self.assert_success(result, codec, raw, decoded)["train"]["report"]
                    self.assertEqual(report["physical_lines"], 5)
                    self.assertEqual(report["non_empty_lines"], 4)
                    self.assertEqual(report["accepted"], 2)
                    self.assertEqual(report["skipped"], 2)
                    self.assertEqual(len(report["rejected_examples"]), 2)
                    offset = 2 + len(good)
                    for number, rejected, example in zip(
                            (3, 4), (malformed, future), report["rejected_examples"]):
                        self.assertEqual(example["line_number"], number)
                        self.assertEqual(example["byte_offset"], offset)
                        self.assertEqual(example["row_sha256"], sha256(rejected))
                        offset += len(rejected)
                    if baseline is None:
                        baseline = decoded_report(report)
                    self.assertEqual(decoded_report(report), baseline)

    def test_ten_thousand_varied_rows_match_across_codecs_and_storage(self):
        rows = training_corpus().splitlines(keepends=True)
        count = 10000
        decoded = b"".join(rows[index % len(rows)] for index in range(count))
        self.assertLess(len(decoded), 8 * 1024 * 1024, "keep the regression corpus bounded")
        chunk_size = 64 * 1024
        boundaries = range(chunk_size, len(decoded), chunk_size)
        self.assertTrue(any(decoded[offset - 1] != 10 for offset in boundaries),
                        "at least one member/frame boundary must split a JSONL row")
        baseline = None
        for codec in ("plain", *sorted(self.codecs)):
            # Both compressed formats cross record boundaries in 64 KiB pieces.
            # zstd uses bounded raw blocks; its separate fixed fixture covers
            # ordinary compressed blocks and checksum verification.
            raw = decoded if codec == "plain" else b"".join(
                encode(decoded[offset:offset + chunk_size], codec)
                for offset in range(0, len(decoded), chunk_size))
            for mode in ("memory", "disk"):
                with self.subTest(codec=codec, mode=mode, records=count):
                    result = self.invoke(raw, mode, SUFFIXES[codec],
                                         extra=("--positions-sha256", sha256(raw)))
                    report = self.assert_success(result, codec, raw, decoded)["train"]["report"]
                    self.assertEqual(report["physical_lines"], count)
                    self.assertEqual(report["non_empty_lines"], count)
                    self.assertEqual(report["accepted"], count)
                    self.assertEqual(report["skipped"], 0)
                    self.assertEqual(report["skipped_by_reason"], {})
                    self.assertEqual(report["rejected_examples"], [])
                    self.assertEqual(report["missing_game_result"], 0)
                    self.assertEqual(report["unknown_game_result"], 0)
                    if baseline is None:
                        baseline = decoded_report(report)
                    self.assertEqual(decoded_report(report), baseline)

    def test_magic_takes_priority_and_extension_fallback_never_reads_plain(self):
        data = benchmark.record_bytes()
        for codec in sorted(self.codecs):
            raw = encode(data, codec)
            for mode in ("memory", "disk"):
                for suffix in (".jsonl", ".zstd" if codec == "gzip" else ".gzip"):
                    with self.subTest(codec=codec, mode=mode, suffix=suffix):
                        self.assert_success(self.invoke(raw, mode, suffix), codec, raw, data)
                for suffix in ((".gz", ".gzip") if codec == "gzip" else (".zst", ".zstd")):
                    for plain in (data, b"", b"\n"):
                        self.assert_ingest_failure(self.invoke(plain, mode, suffix))

    def test_concatenated_members_join_mid_record_and_mid_utf8_character(self):
        row = benchmark.synthetic_record()
        row["source"]["path"] = "synthetic-駒"
        data = json.dumps(row, ensure_ascii=False, separators=(",", ":")).encode() + b"\r\n"
        split = data.index("駒".encode()) + 1
        for codec in sorted(self.codecs):
            # An empty member and splits inside a JSON record must preserve bytes.
            raw = encode(b"", codec) + encode(data[:split], codec) + encode(data[split:], codec)
            for mode in ("memory", "disk"):
                report = self.assert_success(
                    self.invoke(raw, mode, SUFFIXES[codec]), codec, raw, data)["train"]["report"]
                self.assertEqual(report["accepted"], 1)
                self.assertEqual(report["physical_lines"], 1)
                empty = encode(b"", codec)
                empty_report = self.assert_success(
                    self.invoke(empty, mode, SUFFIXES[codec]), codec, empty, b"")["train"]["report"]
                self.assertEqual(empty_report["accepted"], 0)
                self.assertEqual(empty_report["physical_lines"], 0)

    def test_raw_hash_and_validation_hash_contracts(self):
        data = benchmark.record_bytes() * 2
        previous = b'{"previous_complete_manifest":"preserve"}\n'
        for codec in sorted(self.codecs):
            raw = encode(data, codec)
            for mode in ("memory", "disk"):
                wrong = self.invoke(raw, mode, SUFFIXES[codec], previous_manifest=previous,
                                    extra=("--positions-sha256", sha256(data)))
                self.assert_ingest_failure(wrong, previous)
                self.assertIn("SHA-256 mismatch", wrong["stderr"])
                result = self.invoke(data, mode, validation=(raw, SUFFIXES[codec]),
                                     extra=("--positions-sha256", sha256(data),
                                            "--validation-positions-sha256", sha256(raw)))
                document = self.assert_success(result, "plain", data, data)
                self.assert_report_hashes(document["validation"]["report"], codec, raw, data)
                wrong = self.invoke(data, mode, validation=(raw, SUFFIXES[codec]),
                                    extra=("--validation-positions-sha256", sha256(data)))
                self.assert_ingest_failure(wrong)

    def test_truncation_corruption_and_trailing_data_never_replace_manifest(self):
        data = benchmark.record_bytes() * 2
        previous = b'{"previous_complete_manifest":"preserve"}\n'
        for codec in sorted(self.codecs):
            raw = encode(data, codec)
            corruptions = [(f"truncated-{cut}", raw[:cut])
                           for cut in (0, 1, 4, 9, len(raw) // 2, len(raw) - 1)]
            corruptions += [("trailing-garbage", raw + b"garbage"),
                            ("second-frame-truncated", raw + encode(data, codec)[:-1])]
            if codec == "gzip":
                for offset, reason in ((-8, "crc"), (-4, "isize")):
                    damaged = bytearray(raw)
                    damaged[offset] ^= 1
                    corruptions.append((reason, bytes(damaged)))
                    corruptions.append((f"second-frame-{reason}", raw + bytes(damaged)))
            else:
                damaged = bytearray(raw)
                damaged[9] |= 6  # Reserved block type.
                corruptions += [("reserved-block", bytes(damaged)),
                                ("second-frame-reserved-block", raw + bytes(damaged))]
            for mode in ("memory", "disk"):
                for reason, damaged in corruptions:
                    with self.subTest(codec=codec, mode=mode, reason=reason):
                        result = self.invoke(damaged, mode, SUFFIXES[codec],
                                             previous_manifest=previous)
                        self.assert_ingest_failure(result, previous)
                # JSONL strictness remains independent from decoder integrity.
                damaged_jsonl = encode(data + b"broken\n", codec)
                result = self.invoke(damaged_jsonl, mode, SUFFIXES[codec],
                                     extra=("--strict-positions",))
                self.assert_ingest_failure(result)
                # A broken validation stream cannot publish a partial train audit.
                result = self.invoke(data, mode, validation=(raw[:-1], SUFFIXES[codec]))
                self.assert_ingest_failure(result)

    def test_zstd_compressed_blocks_and_checksum(self):
        if "zstd" not in self.codecs:
            self.skipTest("positions-zstd intentionally disabled")
        decoded = benchmark.record_bytes() * 2
        raw = bytes.fromhex(ZSTD_COMPRESSED_FRAME_HEX)
        damaged = bytearray(raw)
        damaged[-1] ^= 1
        for mode in ("memory", "disk"):
            report = self.assert_success(self.invoke(raw, mode), "zstd", raw, decoded)["train"]["report"]
            self.assertEqual(report["accepted"], 2)
            concat = raw + raw
            report = self.assert_success(self.invoke(concat, mode), "zstd", concat, decoded * 2)["train"]["report"]
            self.assertEqual(report["accepted"], 4)
            for bad in (bytes(damaged), raw[:-1], raw + bytes(damaged), raw + b"garbage"):
                with self.subTest(mode=mode, length=len(bad)):
                    self.assert_ingest_failure(self.invoke(bad, mode, ".zst"))

    def test_training_weights_match_and_resume_keeps_physical_identity(self):
        if not self.codecs:
            self.skipTest("no compressed input features enabled")
        decoded = b"".join(training_corpus().splitlines(keepends=True)[:6])
        recipe = ("--epochs", "1", "--lr-schedule-epochs", "1", "--label-depth", "1",
                  "--teacher-eval", "material", "--seed", "42", "--shuffle-seed", "17",
                  "--source-cap", "5", "--validation-ratio", "0.25", "--side-balance",
                  "--wdl-lambda", "0.3", "--resume-checkpoint-every-games", "2")

        def train(codec, mode, extra=(), stopped=False, rejected=False):
            raw = encode(decoded, codec)
            result = self.invoke(raw, mode, SUFFIXES[codec], audit=False,
                                 extra=(*recipe, "--positions-sha256", sha256(raw), *extra))
            if rejected:
                self.assertNotEqual(result["code"], 0, result["stderr"])
                self.assertIn("recipe fingerprint mismatch", result["stderr"])
                self.assertFalse(result["output"].exists())
            else:
                self.assert_success(result, codec, raw, decoded)
                if stopped:
                    self.assertFalse(result["output"].exists())
                    state = json.loads((result["directory"] / "weights.resume.json").read_text())
                    self.assertEqual(state["epoch_completed"], 0)
                    self.assertEqual(state["next_game_index"], 2)
                    self.assertEqual(state["optimizer"]["step"], 2)
                    self.assertGreater(len(state["teacher_cache"]), 0)
                else:
                    self.assertTrue(result["output"].is_file())
            return result

        baseline = None
        baseline_meta = None
        for codec in ("plain", *sorted(self.codecs)):
            for mode in ("memory", "disk"):
                result = train(codec, mode)
                directory = result["directory"]
                metadata = json.loads((directory / "weights.epoch1.meta.json").read_text())
                self.assertEqual(metadata["train_count"], 4)
                self.assertEqual(metadata["valid_count"], 1)
                self.assertEqual(metadata["positions_sha256"], sha256(encode(decoded, codec)))
                hashes = {name: benchmark.file_sha256(directory / name)
                          for name in ("weights.bin", "weights.epoch1.bin", "weights.epoch1.adam.json")}
                if baseline is None:
                    baseline, baseline_meta = hashes, metadata
                self.assertEqual(hashes, baseline, (codec, mode))
                for key in ("checkpoint_hash", "split_hash", "train_count", "valid_count",
                            "cache_hits", "cache_misses", "train_wdl_component"):
                    self.assertEqual(metadata[key], baseline_meta[key], (codec, mode, key))

        partial_plain = train("plain", "memory", ("--stop-after-resume-checkpoint",), stopped=True)
        plain_checkpoint = partial_plain["directory"] / "weights.resume.json"
        for codec in sorted(self.codecs):
            train(codec, "disk", ("--resume-checkpoint", str(plain_checkpoint)), rejected=True)
            partial = train(codec, "memory", ("--stop-after-resume-checkpoint",), stopped=True)
            checkpoint = partial["directory"] / "weights.resume.json"
            resumed = train(codec, "disk", ("--resume-checkpoint", str(checkpoint)))
            for name, expected in baseline.items():
                self.assertEqual(benchmark.file_sha256(resumed["directory"] / name), expected,
                                 (codec, "resumed", name))


if __name__ == "__main__":
    unittest.main()
