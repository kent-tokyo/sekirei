# Optional compressed positions input

`positions-gzip` and `positions-zstd` independently enable compressed JSONL in
`sekirei-train`. Both are off by default. Gzip uses flate2 1.1.10 with the explicit
miniz_oxide backend; zstd uses ruzstd 0.9.1 with checksum computation enabled.
The adapter uses safe Rust and adds no C/C++ decoder bindings. This is not a
claim that all transitive Rust dependencies contain no unsafe code.

[Validation](validation.json) pins the implementation commit, Cargo.lock, test
scripts, release executables, commands and host. The preceding
[storage comparison](../positions_ingestion_v0.4.1/README.md) remains historical
evidence for its original commit; its RSS/timing values were not remeasured here.

## Input and audit contract

The reader dispatches through an enum. Magic takes precedence over the filename;
recognized compressed suffixes also select the decoder, so a truncated header
cannot silently fall back to skipped JSONL. All concatenated members/frames and
zstd skippable frames are consumed. Truncation, checksum errors, unsupported
headers and trailing garbage fail ingestion before a new complete manifest is
published. An existing manifest survives a failed replacement.

Physical-file SHA-256 and byte counts remain authoritative for requested hashes
and resume recipes. Separate decoded SHA-256 and byte counts describe the exact
JSONL. Rejected-row offsets and hashes refer to that decoded stream, including
original line endings. Plain input has the same physical and decoded identity.
Changing memory/disk storage permits resume; recompressing the input changes its
physical identity and requires a new run.

Zstd rejects dictionaries and windows larger than 8 MiB. The window limit is not
a total RSS bound: decoder scratch, a block, the longest JSONL row and existing
training storage add memory. Disk storage still retains O(N) offsets/shuffle
indices and the existing teacher cache. No throughput or strength result is
inferred from these correctness checks.

## Verification

The compression CLI suite compares 10,000 varied synthetic records across
plain/gzip/zstd and memory/disk, including concatenated 64 KiB chunks that split
records. Every decoded audit field matches. A separate fixture covers invalid
JSON, schema versions, blank lines, CRLF and missing final newlines.

A six-record material-teacher recipe tests source caps, seeded shuffle, splitting
and WDL mixing. Final weights, epoch weights and Adam state are byte-identical
across all six input/storage combinations. Mid-epoch resume from memory to disk
also matches for each codec; a transport change is rejected by the existing
recipe fingerprint. These are small regression fixtures, not trained candidates.

Raw zstd test frames are generated with Python standard-library byte operations.
A pinned ordinary compressed-block fixture, generated with Pure Rust ruzstd,
separately checks real compressed blocks and checksums. Its encoded/decoded hashes
and generation parameters are recorded in the test script and validation record.
Only fresh synthetic data is used; existing corpora, hold-outs and training jobs
are outside this work.

Local results: 963 default workspace Rust tests passed (4 ignored), 273 trainer
package tests passed with both codecs, and all 11 compressed-input CLI/helper
tests passed. The default build ran 9 of those tests with 2 intentional skips;
its feature-disabled errors were verified. The 12 existing ingestion tests passed
with both the default and codec-enabled release binaries. Single-codec builds,
Clippy and formatting passed.

The unchanged 98% engine/library coverage gate passed at **40,797 / 41,553 lines
(98.180636777128%)**. The unfiltered workspace figure is 86.14353815547464% and is
only a diagnostic. [Coverage reports](coverage/summary.json) and compressed raw
[LCOV evidence](coverage/lcov.info.gz) are retained. An initial disk-capacity
failure is logged separately; the complete retry used fresh profiles after
removing only this worktree's regenerable debug cache. Final coverage reports
and LCOV were retained before removing its coverage build cache. CI now runs default-off and enabled-codec contracts; hosted CI has not run for this local
branch. Optional codec paths are tested separately from the existing default
coverage gate.

## Reproduce

```bash
cargo build --release -p sekirei-train --bin train \
  --features positions-gzip,positions-zstd
TRAIN_BIN="$PWD/target/release/train" POSITIONS_CODECS=gzip,zstd \
  python3 -B scripts/test_positions_compression.py
TRAIN_BIN="$PWD/target/release/train" python3 -B scripts/test_positions_ingest.py
cargo test -p sekirei-train --features positions-gzip,positions-zstd
cargo test --workspace
bash scripts/check_rust_coverage.sh
```

Rebuild without codec features and set `POSITIONS_CODECS=` to test disabled-codec
errors. The recorded local runs disable incremental builds and debug information,
use two build jobs and a 16 MiB test stack, and run the default workspace tests
serially. Release builds retain native CPU targeting and the repository profile.
