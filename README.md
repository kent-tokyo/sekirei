# Sekirei — Rust Shogi Engine

[![CI](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml/badge.svg)](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml)
[![Release](https://img.shields.io/badge/release-v0.3.38-blue)](https://github.com/kent-tokyo/sekirei/releases/tag/v0.3.38)
[![crates.io](https://img.shields.io/crates/v/sekirei.svg)](https://crates.io/crates/sekirei)
[![License](https://img.shields.io/crates/l/sekirei.svg)](https://github.com/kent-tokyo/sekirei/blob/main/LICENSE)

[日本語](README_ja.md)

Sekirei is an experimental shogi engine written in pure Rust. Release `0.3.38`
provides a USI engine, CSA client, match runner, NNUE trainer, and reusable core
library. Local diagnostics are not absolute rating claims.

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
Without a checkpoint the engine uses material evaluation. The recommended
0.3.38 checkpoint is versioned separately at
[`weights/sekirei-nnue-v0.3.38.bin`](weights/sekirei-nnue-v0.3.38.bin):

```bash
sekirei /path/to/sekirei-nnue-v0.3.38.bin
```

Verify the SHA-256 and license in [the weight artifact card](weights/README.md)
before use. NNUE weights are not bundled with the crate; this keeps the
MIT/Apache source package and CC BY 4.0 model artifact separate.

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

`SearchMode=Speculative` is the default; parallel modes may be
nondeterministic. Use `Threads=1` and `SpecTopN=0` for deterministic checks.
Weight validation, model format, and the limited external-SFNN boundary are
documented in [NNUE weights](docs/nnue_weights.md).

## Build and verify

```bash
cargo build --release
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Benchmark and cross-library commands, their pinned inputs, and historical
scope are indexed in [scripts/README.md](scripts/README.md). A component timing
is not an overall speed or playing-strength ranking.

## Matches and CSA/Floodgate

Use `sekirei-match` for local USI matches. For unattended offline self-play
with durable records:

```bash
python3 scripts/run_local_selfplay.py --games 1000 \
  --weights /path/to/weights.bin \
  --positions data/gate/openings_standard.sfen
```

Each run stores a manifest, kifu, CSA, per-move search data, and deduplication
metadata below ignored `data/runs/`. Same-engine self-play is training and
regression data, not an Elo claim.

`sekirei-csa` provides CSA/Floodgate play. Inject credentials at runtime and
never commit credentials, game records, generated weights, or training data.

Self-play Elo is relative to the selected baseline and is not a Floodgate or
human rating. Statistical gates distinguish `PASS`, `FAIL`, and
`INCONCLUSIVE`.

## NNUE training

Run `cargo run --release -p sekirei-train -- --help` for the training CLI.
Reproducible recipes, diagnostics, and resume tooling are indexed in
[scripts/README.md](scripts/README.md); generated training artifacts remain
outside Git.

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
