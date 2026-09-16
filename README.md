# Sekirei — Rust Shogi Engine

[![CI](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml/badge.svg)](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml)
[![Release](https://img.shields.io/badge/release-v0.3.37-blue)](https://github.com/kent-tokyo/sekirei/releases/tag/v0.3.37)
[![crates.io](https://img.shields.io/crates/v/sekirei.svg)](https://crates.io/crates/sekirei)
[![License](https://img.shields.io/crates/l/sekirei.svg)](https://github.com/kent-tokyo/sekirei/blob/main/LICENSE)

[日本語](README_ja.md)

Sekirei is an experimental shogi engine written in pure Rust. Release `0.3.37`
provides a USI engine, CSA/Floodgate client, match runner, NNUE training tools,
and reusable core library. Playing strength is still under development; local
diagnostics and self-play results are not absolute rating claims.

## Quick start

Install the released USI engine:

```bash
cargo install sekirei
sekirei
```

Or run the current checkout:

```bash
git clone https://github.com/kent-tokyo/sekirei.git
cd sekirei
cargo run --release -p sekirei
```

Register the resulting `sekirei` executable in a USI-compatible shogi GUI.
Without a checkpoint the engine uses material evaluation. To enable NNUE:

```bash
sekirei /path/to/weights.bin
```

NNUE weights are not bundled with the crate.

## What is included

- Complete shogi rules, legal move generation, drops, promotions, SFEN, and
  USI move notation.
- Iterative-deepening alpha-beta/PVS search, quiescence search, pruning,
  move ordering, and a lock-free transposition table.
- Optional speculative search and Lazy SMP. Use `SpecTopN=0`, `Threads=1`
  for deterministic diagnostics.
- Incrementally updated NNUE-style evaluation and a reproducible trainer.
- Opt-in bounded root MCTS and df-pn mate-search APIs. These are experimental
  capabilities, not strength claims.
- CSA v2.2/Floodgate play and USI-vs-USI match tooling.
- Pure Rust core search and evaluation code with no `unsafe` blocks.

Workspace binaries:

| Command | Package | Purpose |
|---|---|---|
| `sekirei` | `sekirei` | USI engine |
| `sekirei-csa` | `sekirei-csa` | CSA/Floodgate client |
| `sekirei-match` | `sekirei-match-runner` | USI match runner |
| `train` | `sekirei-train` | NNUE training |

## Engine configuration

The engine reports its complete option list after the USI `usi` command. The
main options are `Hash`, `Threads`, `MoveOverhead`, `Ponder`, `MultiPV`,
`EvalFile`, `SearchMode`, `SpecTopN`, and the opening-book options.

`SearchMode=Speculative` is the default. `SearchMode=LazySMP` runs independent
workers with private boards and heuristics while sharing the transposition
table and stop flag. Scheduling can make parallel searches nondeterministic.

Check a weight file without installing it globally:

```bash
cargo run --release -p sekirei-bench --bin nnue_probe -- \
  /path/to/weights.bin --strict --json
```

The strict probe detects constant or nearly constant output, missing material
or side-to-move sensitivity, collapsed layers, and nondeterministic reloads.
See [NNUE weights](docs/nnue_weights.md) for the model and license boundary.
External SFNN support currently stops at bounded header and provenance
inspection; accepted metadata does not imply inference compatibility.

## Build, test, and benchmark

```bash
cargo build --release
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo bench --bench movegen -p sekirei-bench
```

For the pinned cross-library diagnostic:

```bash
cargo run --release -p sekirei-bench --bin cross_library -- --check
cargo run --release -p sekirei-bench --bin cross_library -- --components
```

The v0.3.35 ten-session comparison measured a combined rsshogi/Sekirei ratio
of 1.1596x (95% CI 1.1364–1.1833x) on three shared legal-move-generation
cases. It excludes full-state updates, NNUE, search, and playing strength, so
it is not an overall speed ranking. The corresponding
[ten-session report](scripts/benchmark_reports/cross_library_component_10session_2026-09-12.md)
records the host and measurement boundary.

## Matches and CSA/Floodgate

Run a local match:

```bash
cargo run --release -p sekirei-match-runner -- \
  --engine1 ./target/release/sekirei \
  --engine2 /path/to/other-engine \
  --games 100 --byoyomi 10000 \
  --positions data/gate/openings_standard.sfen \
  --games-per-position 4 --json results/run.json
```

Run the CSA client:

```bash
cargo run --release -p sekirei-csa -- \
  --user <name> --trip <secret> --game floodgate-300-10F \
  --record-dir data/floodgate --loop
```

`FLOODGATE_ACCOUNT` and `FLOODGATE_TRIP` may be injected through the
environment instead of command-line arguments. Do not commit credentials,
game records, generated weights, or training data. `--analysis-dir <dir>`
adds schema-versioned per-move diagnostic sidecars; missing historical values
must remain missing rather than being reconstructed as observations.

Self-play Elo is relative to the selected baseline and is not a Floodgate or
human rating. Statistical gates distinguish `PASS`, `FAIL`, and
`INCONCLUSIVE`.

## NNUE training

```bash
cargo run --release -p sekirei-train -- --help
cargo run --release -p sekirei-train -- \
  --games /path/to/csa_dir --output weights.bin --epochs 3
```

The trainer supports fixed NNUE teachers, deterministic node-budget labels,
validation splits, Adam state, and full mid-epoch resume checkpoints. Training
artifacts remain separate from inference `.bin` files. Detailed operational
entry points are indexed in [scripts/README.md](scripts/README.md).

## Documentation

- [Changelog](CHANGELOG.md)
- [NNUE weights and licensing](docs/nnue_weights.md)
- [Analysis benchmark contract](docs/amateur_analysis_benchmark.md)
- [Mobile integration status](docs/mobile_integration.md)
- [Script and validation-tool index](scripts/README.md)

Implementation, tests, measurements, and released artifacts are separate
claims. Historical experiment reports preserve their original version and
scope; they are not statements about the current engine unless explicitly
revalidated.

## License and attribution

Source code is available under [MIT](LICENSE-MIT) or
[Apache-2.0](LICENSE-APACHE), at your option. Retain [NOTICE](NOTICE), which
contains the Sekirei and Kentaro Tanabe attribution. NNUE weight files are
separate artifacts licensed under CC BY 4.0; see
[NNUE-LICENSE.md](NNUE-LICENSE.md).

Recommended attribution:

```text
This product is based on Sekirei,
an open-source shogi engine developed by Kentaro Tanabe.

https://github.com/kent-tokyo/sekirei
```

This display is recommended, not an additional advertising clause. Do not use
the Sekirei name or logo to imply official endorsement without permission.
