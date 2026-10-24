# Sekirei — Rust Shogi Engine

[![CI](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml/badge.svg)](https://github.com/kent-tokyo/sekirei/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/kent-tokyo/sekirei/branch/main/graph/badge.svg?flag=rust-engine)](https://codecov.io/gh/kent-tokyo/sekirei)
[![Ask DeepWiki](docs/assets/ask-deepwiki.svg)](https://deepwiki.com/kent-tokyo/sekirei)
[![Release](https://img.shields.io/badge/release-v0.5.1-blue)](https://github.com/kent-tokyo/sekirei/releases/tag/v0.5.1)
[![crates.io](https://img.shields.io/crates/v/sekirei.svg)](https://crates.io/crates/sekirei)
[![License](https://img.shields.io/crates/l/sekirei.svg)](https://github.com/kent-tokyo/sekirei/blob/main/LICENSE)

[日本語](README_ja.md)

Sekirei is an experimental Pure Rust shogi engine. Release `0.5.1` uses
material evaluation by default. Optional NNUE weights require an explicit
file path; see [weights and evidence](docs/nnue_weights.md). Release changes
are in the [changelog](CHANGELOG.md).

## Quick start

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

Register `sekirei` in a USI-compatible GUI. Without `EvalFile`, the engine
uses material evaluation. External HalfKP 256x2-32-32 files use `FV_SCALE`
and remain subject to their own licenses. The optional `nn_r6` weight needs
`FV_SCALE=24`; its pinned 1T/4T material gates do not establish an external
rating or general playing-strength gain. Loading and evidence details are in
[NNUE weights](docs/nnue_weights.md).

## Components

- Shogi rules, SFEN/USI notation, alpha-beta/PVS, quiescence search, move
  ordering, and a lock-free transposition table.
- Optional speculative search, Lazy SMP, NNUE training/evaluation, MCTS, and
  df-pn. Experimental modes are not strength claims.
- CSA/Floodgate and local USI match tooling. Core search and evaluation use no
  `unsafe` code.

| Command | Package | Purpose |
|---|---|---|
| `sekirei` | `sekirei` | USI engine |
| `sekirei-csa` | `sekirei-csa` | CSA/Floodgate client |
| `sekirei-match` | `sekirei-match-runner` | USI match runner |
| `train` | `sekirei-train` | NNUE training |

The `usi` command lists every engine option. `SearchMode=Auto` selects
sequential search for one worker and Lazy SMP for multiple workers. For
deterministic diagnostics, use `Threads=1` and `SpecTopN=0`.

### Browser / WebAssembly

[`sekirei-wasm`](crates/sekirei-wasm/README.md) provides SFEN parsing, legal
moves, validated move application, bounded one-worker search with an optional
caller-supplied evaluator, up to four MultiPV lines, and bounded shortest-mate
analysis. The prebuilt v0.5.1 ES-module
package is attached to the
[GitHub Release](https://github.com/kent-tokyo/sekirei/releases/download/v0.5.1/sekirei-wasm-0.5.1.tgz).

## Build and verify

```bash
cargo build --release
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
bash scripts/check_rust_coverage.sh
python3 scripts/check_release_metadata.py --allow-planned-release-manifest
python3 scripts/check_release_manifest_layout.py
```

The coverage command requires `cargo-llvm-cov 0.8.7`,
`llvm-tools-preview`, and `jq`; it enforces the 98% line-coverage gate.
The current release manifest stays in the repository root and older ones in
the [archive](docs/releases/manifests/README.md). CI checks published
`sekirei-core` API compatibility against v0.3.73.

The [8T](docs/experiments/compiler_pgo_8t_v0.4.2/README.md),
[4T](docs/experiments/compiler_pgo_4t_v0.4.1/README.md), and
[own-HalfKP 8T](docs/experiments/own_halfkp_pgo_8t_v0.4.4/README.md)
PGO reports give the pinned binaries, workloads, and uncertainty. These
measurements do not describe ordinary Cargo builds or playing strength.

## Matches and training

Use `sekirei-match` for local USI matches. Durable offline self-play requires
explicit weights and opening positions:

```bash
python3 scripts/run_local_selfplay.py --games 1000 \
  --weights /path/to/weights.bin \
  --positions data/gate/openings_standard.sfen
```

Runs store manifests, records, CSA, and search data under ignored `data/runs/`.
Same-engine results are regression evidence, not Floodgate or human ratings.
See the [script index](scripts/README.md) for match and gate tooling.

For NNUE training, run `cargo run --release -p sekirei-train -- --help`.
Data and weights remain outside Git. Recipes, hashes, resume rules, and
whole-game split requirements are in the [script index](scripts/README.md).

Opening-book support is build-time and runtime opt-in:

```bash
cargo build --release -p sekirei --features opening-book
# Then set BookFile and UseBook=true.
```

No default book is shipped. Book and training workflows retain hashed
manifests so inputs, settings, and outputs can be audited.

## Documentation

- [Documentation index](DOCUMENTATION.md)
- [Changelog](CHANGELOG.md) and [archived history](CHANGELOG_ARCHIVE.md)

Implementation, tests, measurements, and released artifacts are separate
claims. Historical reports keep their original version and scope unless they
are explicitly revalidated.

## License and attribution

Source code is [MIT](LICENSE-MIT) OR [Apache-2.0](LICENSE-APACHE); retain
[NOTICE](NOTICE). NNUE files are separate CC BY 4.0 artifacts; see
[NNUE-LICENSE.md](NNUE-LICENSE.md). The attribution text in `NOTICE` is
recommended when redistributing Sekirei, but it is not an additional
advertising clause. Do not imply official endorsement without permission.
