# Sekirei — Rust Shogi Engine

[![CI](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml/badge.svg)](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/sekirei.svg)](https://crates.io/crates/sekirei)
[![License](https://img.shields.io/crates/l/sekirei.svg)](https://github.com/kent-tokyo/sekirei/blob/main/LICENSE)

[日本語](README_ja.md)

Sekirei is an experimental **shogi (Japanese chess) engine written in Rust** (current release:
`0.3.28`). It speaks the
Universal Shogi Interface (USI) protocol used by shogi GUIs, supports CSA/Floodgate games, and
includes NNUE-style evaluation, parallel alpha-beta search, and tools for self-play strength
testing.

This project is for developers interested in Rust game engines, shogi search, safe parallelism,
NNUE evaluation, and reproducible engine experiments. Playing strength and evaluation quality
are still under development; Sekirei makes no absolute rating or superiority claim.

## Quick start

Install the USI engine from crates.io:

```bash
cargo install sekirei
sekirei
```

Or build the latest source checkout:

```bash
git clone https://github.com/kent-tokyo/sekirei.git
cd sekirei
cargo run --release -p sekirei
```

The binary reads USI commands from standard input. To connect Sekirei to a compatible shogi GUI,
select the installed `sekirei` executable as the engine command. Sekirei can run without a
checkpoint using its material-evaluation fallback; pass an NNUE weight file as the first argument
to enable a trained evaluator:

```bash
sekirei /path/to/weights.bin
```

## Features

- Rust shogi engine with a 9×9 board, legal move generation, promotion, drops, SFEN, and USI moves.
- USI engine binary for shogi GUI integration and command-line analysis.
- Iterative deepening, negamax/alpha-beta search, PVS/YBW parallel search, quiescence search,
  move ordering, and pruning heuristics.
- Lock-free transposition table, optional speculative parallel search, and an
  opt-in Lazy SMP search backend.
- An opt-in deterministic root MCTS pilot with injectable policy/value providers.
- An opt-in bounded df-pn mate-search API with node/depth limits and safe
  `Unknown` results when the configured boundary is reached.
- NNUE-style efficiently updatable evaluation with file-based checkpoints.
- CSA v2.2 / Floodgate client for automated games.
- USI-vs-USI match runner for self-play, regression testing, and relative Elo estimation.
- NNUE training pipeline from CSA games or extracted positions.
- Pure Rust core logic with no `unsafe` blocks in the core search and evaluation code.

## Status

- Pure Rust; the core search and evaluation code contains no `unsafe`.
- USI engine binary: `sekirei`.
- CSA client binary: `sekirei-csa`.
- Match runner binary: `sekirei-match`.
- NNUE training binary: `train` (package: `sekirei-train`).
- NNUE weights are loaded from a file and are not bundled.

The `sekirei` package on crates.io is the USI engine binary. The repository is a Cargo workspace
that also publishes the reusable `sekirei-core` library and supporting command-line tools.

## Repository layout

```text
crates/sekirei-core/         board, move generation, search, TT, evaluation
crates/sekirei-usi/          USI engine (`sekirei`)
crates/sekirei-csa/          CSA/floodgate client (`sekirei-csa`)
crates/sekirei-match-runner/ USI-vs-USI match runner (`sekirei-match`)
crates/sekirei-train/        NNUE training pipeline (`train`)
crates/sekirei-bench/        benchmarks
scripts/                     training and strength-test helpers
```

The core currently includes alpha-beta/negamax, PVS/YBW parallel search, iterative deepening,
quiescence search, a lock-free transposition table, common move-ordering and pruning heuristics,
optional speculative search, and an opt-in Lazy SMP backend. `SpecTopN=0` disables speculative
search and is useful when a repeatable run is required. A verification search used by singular extensions is deliberately
excluded from unrestricted TT writes, so a partial verification result cannot overwrite the
parent node's reusable entry.

The core also contains experimental root-level MCTS and bounded df-pn APIs.
They are opt-in research components, are not wired into the default USI mode,
and do not establish a playing-strength result.

For a bounded mate probe, the USI option `SearchMode=Dfpn` selects the df-pn
backend. It is intentionally opt-in, uses the requested `depth` as its ply
boundary, and should not be treated as the default playing mode or as a
strength comparison.

## Build and test

```bash
cargo build --release
cargo test --release
cargo bench --bench movegen -p sekirei-bench
```

### Local performance snapshot

The v0.3.27 hot-path pass reduced the median start-position timings on the development Mac from
8.2711 us to 2.2151 us for legal move generation, from 9.2530 ms to 2.1082 ms for Perft(3), and
from 22.544 ms to 7.659 ms for depth-4 search. The longer search sample used 20 measurements.
These are local mechanism diagnostics on heterogeneous Apple CPU cores, not portable performance,
playing-strength, or Elo claims.

Probe an NNUE checkpoint without enabling process-global engine weights:

```bash
cargo run --release -p sekirei-bench --bin nnue_probe -- /path/to/weights.bin
# For automation, add --json; custom positions use repeated --sfen "<SFEN>".
```

The probe reports evaluator scores, score range, mean, variance, and reference
deltas, plus `constant_output` and `reload_deterministic` flags. Add `--json`
for machine-readable output; `--strict` exits non-zero for constant or
near-constant output (range below 8 cp) or non-deterministic reload. It is a
diagnostic, not a strength test.
JSON output also includes `strict_min_range_cp` and `strict_pass` so automated
candidate selection can record the exact health rule used.

Checkpoint files are inference-compatible when loaded by `nnue_probe` or `EvalFile`. The inference
`.bin` remains optimizer-free; training emits separate Adam and full-resume sidecars.

Run the USI engine without weights (material evaluation fallback):

```bash
cargo run --release -p sekirei
```

Run it with NNUE weights:

```bash
cargo run --release -p sekirei -- /path/to/weights.bin
```

Print the version without starting the USI loop:

```bash
cargo run --release -p sekirei -- --version
```

Use `--help` for a concise usage summary.

## USI options

The engine reports the complete option list after `usi`. The main options are:

- `Hash`, `Threads`, `MoveOverhead`
- `SearchMode` (`Speculative` by default; optional `LazySMP`)
- `Ponder`, `MultiPV`
- `EvalFile` (loaded on `isready`)
- `SpecTopN` (default `3`; `0` disables speculation)
- `UseBook`, `BookFile`, `BookMaxPly`, `BookMinConfidence`

With `SpecTopN > 0`, scheduling of speculative tasks can make otherwise identical searches
nondeterministic. Use `SpecTopN=0` for deterministic comparisons where practical. For correctness
diagnostics, keep `Threads=1`, `Parallel=1`, and `SpecTopN=0`; this control is separate from
timing and match-strength measurements.

In `SearchMode=LazySMP`, `Threads` selects the number of independent workers. Workers use private
boards and heuristic tables while sharing the lock-free transposition table and stop flag. This
mode remains opt-in; the default is `SearchMode=Speculative`.

## CSA / floodgate

```bash
cargo run --release -p sekirei-csa -- \
  --user <name> --trip <secret> --game floodgate-300-10F --loop
```

`FLOODGATE_ACCOUNT` and `FLOODGATE_TRIP` may be used instead of command-line credentials.
Do not commit credentials, game records, weights, or generated training data.

## Match testing

```bash
cargo run --release -p sekirei-match-runner -- \
  --engine1 ./target/release/sekirei \
  --engine2 /path/to/other-engine \
  --games 100 --byoyomi 10000 \
  --positions data/gate/openings_standard.sfen \
  --games-per-position 4 --json results/run.json
```

Use `gate` on a result JSON to evaluate an existing match. Self-play Elo is relative to the
selected baseline and is not a floodgate rating.

## NNUE training

The training command accepts CSA games or pre-extracted positions. For all options, run:

```bash
cargo run --release -p sekirei-train -- --help
```

Example:

```bash
cargo run --release -p sekirei-train -- \
  --games /path/to/csa_dir --output weights.bin --epochs 3
```

Teacher-search leaves use material evaluation by default. To run a
fixed-teacher/self-distillation experiment, select one immutable checkpoint:

```bash
cargo run --release -p sekirei-train -- \
  --games /path/to/csa_dir --output student.bin --epochs 3 \
  --teacher-eval nnue --teacher-weights teacher.bin
```

The teacher weight hash is included in teacher-cache entries, complete-resume fingerprints, and
checkpoint metadata. A cache or resume checkpoint from another teacher is rejected instead of
silently mixing label sources. This option defines an experiment contract; it is not by itself a
strength claim.

Use `--label-time-ms N` to place a hard wall-clock limit on each cache-miss teacher search when
fixed-depth labeling has pathological outliers. The limit is part of the cache identity, resume
fingerprint, and checkpoint metadata, so bounded labels cannot be mixed with unlimited labels.
For reproducible single-thread labeling, prefer `--label-nodes N`: it applies a deterministic node
budget and is bound into the same cache/resume/metadata contract without depending on host load.

Training data, checkpoints, weights, match output, and experiment logs are local artifacts and
are intentionally excluded from the public repository. NNUE weight files produced for this
project are licensed separately under CC BY 4.0; see [NNUE-LICENSE.md](NNUE-LICENSE.md).

Epoch checkpoints also write a training-only `.adam.json` sidecar containing raw f32 parameters,
Adam moments, and the optimizer step. Resume that state with `--resume-adam`; the inference `.bin`
file remains separate and compatible with the engine. A diagnostic classification can be attached
to a release-manifest-shaped copy without modifying the original:

```bash
python3 scripts/classify_evaluator_failure.py diagnostic.json \
  --manifest release-manifest-v0.3.24.json \
  --output release-manifest-v0.3.24-diagnostic.json
```

Validate the operational fixture or a generated copy with
`python3 scripts/validate_release_manifest.py scripts/fixtures/release_manifest_diagnostic_v1.json`.
For an epoch-boundary full training resume, use `--resume-checkpoint`; it restores the raw weights,
Adam state, completed epoch, data cursor, and recipe fingerprint and rejects recipe mismatches. Set
`--resume-checkpoint-every-games N` to persist an atomic mid-epoch cursor (for positions mode, N is
the position chunk size); the teacher cache is included so a resumed run does not silently change labels.
The small end-to-end regression is available as `bash scripts/test_resume_cli_fixture.sh`.
Resume verification lineage can be recorded with
`python3 scripts/record_resume_run.py --checkpoint run.resume.json --log run.log --dataset data.jsonl --output resume-manifest.json`.
The generated artifact uses the `sekirei.resume-manifest.v1` schema and keeps checkpoint/log hashes separate.
Attach verified resume evidence to a release-manifest copy with
`python3 scripts/attach_resume_manifest.py --release-manifest release-manifest-v0.3.24.json --resume-manifest resume-manifest.json --output release-manifest-with-resume.json`.
The source release manifest is not modified.
The attached `resume_verification.artifacts` list identifies the checkpoint and execution log separately.

For a controlled interruption at an atomic checkpoint boundary:

```bash
cargo run -p sekirei-train -- --positions positions.jsonl --epochs 20 \
  --checkpoint-dir checkpoints --output weights.bin \
  --resume-checkpoint-every-games 1000 --stop-after-resume-checkpoint
cargo run -p sekirei-train -- --positions positions.jsonl --epochs 20 \
  --output weights.bin --resume-checkpoint weights.resume.json
```

Resume rejects an unsupported schema, missing or malformed optimizer state,
non-finite values, a recipe fingerprint mismatch, a cursor beyond the current
epoch, simultaneous `--resume-adam` and `--resume-checkpoint`, or a target
epoch that has already been completed.

## License and attribution

The Sekirei source code is licensed under the MIT License or the Apache License, Version 2.0,
at your option: [LICENSE-MIT](LICENSE-MIT) or [LICENSE-APACHE](LICENSE-APACHE). Please retain
[NOTICE](NOTICE), including the copyright and attribution notice.

Recommended attribution for products based on Sekirei:

```text
This product is based on Sekirei,
an open-source shogi engine developed by Kentaro Tanabe.

https://github.com/kent-tokyo/sekirei
```

If a product has a Legal Notices screen, the attribution above is a suitable display. The
attribution is strongly recommended, but it is not an advertising requirement of the standard
licenses. Do not use the Sekirei name or logo to imply official endorsement or approval without
permission. NNUE weight files are separate artifacts and are licensed under CC BY 4.0 as
described in [NNUE-LICENSE.md](NNUE-LICENSE.md).

The current release record is kept in
[`release-manifest-v0.3.28.json`](release-manifest-v0.3.28.json). The current Lazy SMP USI smoke
transcript is [`scripts/fixtures/usi_smoke_v0.3.28.txt`](scripts/fixtures/usi_smoke_v0.3.28.txt).
These are release-audit evidence, not strength claims.

For an opt-in MCTS candidate diagnostic, create a validated manifest copy without modifying the
release record:

```bash
python3 scripts/record_mcts_manifest.py \
  --release-manifest release-manifest-v0.3.28.json \
  --output candidate-manifest.json --mode SharedMcts \
  --simulations 4 --arena-nodes 31 --transposition-hits 0
```

When the counts are present in a captured USI transcript, they can be extracted without manual
copying:

```bash
python3 scripts/record_mcts_transcript.py \
  --release-manifest release-manifest-v0.3.28.json \
  --transcript shared-mcts-transcript.txt --output candidate-manifest.json
```

Before a release, check the public metadata without compiling or running the engine:

```bash
python3 scripts/check_release_metadata.py
```

This verifies that all crate manifests, `Cargo.lock`, the changelog, the English and Japanese
README, and the license/attribution files agree on the current version.
