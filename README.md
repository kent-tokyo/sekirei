# Sekirei — Rust Shogi Engine

[![CI](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml/badge.svg)](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/sekirei.svg)](https://crates.io/crates/sekirei)
[![License](https://img.shields.io/crates/l/sekirei.svg)](https://github.com/kent-tokyo/sekirei/blob/main/LICENSE)

[日本語](README_ja.md)

Sekirei is an experimental **shogi (Japanese chess) engine written in Rust**. It speaks the
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
cargo run --release -p sekirei-usi
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
- Lock-free transposition table and optional speculative parallel search.
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
and optional speculative search. `SpecTopN=0` disables speculative search and is useful when a
repeatable run is required.

## Build and test

```bash
cargo build --release
cargo test --release
cargo bench --bench movegen -p sekirei-bench
```

Probe an NNUE checkpoint without enabling process-global engine weights:

```bash
cargo run --release -p sekirei-bench --bin nnue_probe -- /path/to/weights.bin
# For automation, add --json; custom positions use repeated --sfen "<SFEN>".
```

The probe reports evaluator scores, score range, mean, variance, and reference
deltas, plus a `constant_output` flag when every probe score is identical. Add
`--json` for machine-readable output. It is a diagnostic, not a strength test.

Run the USI engine without weights (material evaluation fallback):

```bash
cargo run --release -p sekirei-usi
```

Run it with NNUE weights:

```bash
cargo run --release -p sekirei-usi -- /path/to/weights.bin
```

## USI options

The engine reports the complete option list after `usi`. The main options are:

- `Hash`, `Threads`, `MoveOverhead`
- `Ponder`, `MultiPV`
- `EvalFile` (loaded on `isready`)
- `SpecTopN` (default `3`; `0` disables speculation)
- `UseBook`, `BookFile`, `BookMaxPly`, `BookMinConfidence`

With `SpecTopN > 0`, scheduling of speculative tasks can make otherwise identical searches
nondeterministic. Use `SpecTopN=0` for deterministic comparisons where practical.

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

Training data, checkpoints, weights, match output, and experiment logs are local artifacts and
are intentionally excluded from the public repository.

## License

Licensed under either the Apache License, Version 2.0 or the MIT license, at your option.
