# JSONL ingestion storage comparison

The opt-in `--positions-storage disk` path stores accepted raw rows in an
anonymous temporary file and decodes one position at a time. The default remains
`memory`. Both modes publish an ingestion audit after complete input reads and
requested hash checks; the audit covers counts before sample caps and splits.

Implementation: `698dde82a8325653a9aa8608d6b3d1b2b9d5c15f`. This compares two storage
modes of that implementation, not performance against an earlier release.

## Observed ingestion footprint

Apple M4 (10 logical CPUs), macOS 27.0.1, arm64, Rust 1.97.0 / LLVM 22.1.6.
The release build uses native CPU targeting, opt-level 3, Thin LTO, one codegen
unit and abort-on-panic. The serial ingestion path runs with `--positions-audit-only`,
which exits before evaluator initialization and training.

Each corpus uses repeated schema-11 start positions without teacher observations.
The inputs are 2,720,000 and 27,200,000 bytes. Each size has two memory/disk pairs
in ABBA order, with identical binary and input hashes. Values below are the
median of two processes per mode; MiB means 1,048,576 bytes.

| Accepted rows | Storage | Maximum RSS (MiB) | Wall time (s) |
| ---: | --- | ---: | ---: |
| 10,000 | memory | 36.89 | 0.0702 |
| 10,000 | disk | 4.09 | 0.0674 |
| 100,000 | memory | 318.44 | 0.5206 |
| 100,000 | disk | 4.81 | 0.5286 |

All eight runs accepted every row, skipped none, and produced identical ingest
reports for each input. At 100,000 rows, the measured maximum RSS is about 98.5%
lower in disk mode. Process RSS excludes the operating system's filesystem cache;
disk mode also writes an approximately 27.2 MB temporary snapshot for that input.

Wall time includes startup, parsing, board validation, hashing, sample storage,
and manifest I/O. Page cache state was not controlled. Other compiler activity
was observed on this shared host, and the processes are short. These timings are
diagnostics and do not establish a throughput improvement.

## Reproduce

```bash
CARGO_INCREMENTAL=0 cargo build --release -p sekirei-train --bin train
python3 -B scripts/benchmark_positions_ingest.py --binary target/release/train \
  --output data/runs/positions-ingest-10k-repro --records 10000 --repeats 2
python3 -B scripts/benchmark_positions_ingest.py --binary target/release/train \
  --output data/runs/positions-ingest-100k-repro --records 100000 --repeats 2
TRAIN_BIN="$PWD/target/release/train" python3 -B scripts/test_positions_ingest.py
```

Use a new output directory for each invocation. The generator creates only fresh
synthetic inputs. [10,000-row evidence](10k/report.json) and
[100,000-row evidence](100k/report.json) retain executable/input hashes, exact
commands, host details and per-run results. Adjacent manifests and `/usr/bin/time`
logs preserve the raw measurements. Historical absolute paths identify the local
run; recreate the corpus with the pinned generator instead of relying on them.

## Compatibility and scope

[Validation](validation.json) records 265 passing Rust trainer-package tests,
12 passing Python helper/CLI tests, workspace all-target Clippy and formatting.
The CLI tests compare byte-identical final/epoch weights and Adam state across
storage modes, including a mid-epoch resume from memory into disk with the same
checkpoint cadence. They also cover invalid records, raw offsets and hashes,
strict/hash/read failures and audit-output path collisions. Hosted CI, the full
workspace test suite and the 98% coverage gate have not been run for this branch.

The disk mode preserves source caps, the existing SFEN split, exact seeded shuffle,
teacher-cache visibility and resume identity. Its snapshot is stable if the original
input is changed or removed after ingestion. Offset and shuffle vectors still use
O(N) memory, as does the existing teacher cache. Shuffled disk epochs seek and parse
rows again; their throughput is unmeasured. This is not constant-memory training.

No existing training corpus, hold-out, NNUE weights or candidate jobs were used.
The HalfKP packed-data trainer is unchanged. Compression readers and Python APIs
remain separate work. These results make no search or playing-strength claim.
